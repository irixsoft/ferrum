mod encrypt;
mod vapid;

use crate::events::{Event, Kind, PREFS};
use crate::state::State;
use crate::{secrets, setup, time};
use anyhow::{Context, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::SecretKey;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

const VAPID_SETTING: &str = "push.vapid";
const TTL_SECS: u32 = 86_400;
const SEND_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) fn random_secret() -> SecretKey {
    loop {
        if let Ok(key) = SecretKey::from_slice(&rand::random::<[u8; 32]>()) {
            return key;
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Device {
    pub id: String,
    #[serde(skip)]
    pub user_id: String,
    #[serde(skip)]
    pub endpoint: String,
    #[serde(skip)]
    pub p256dh: String,
    #[serde(skip)]
    pub auth: String,
    pub created_at: String,
    pub last_ok_at: Option<String>,
}

/// What `PushSubscription.toJSON()` gives the panel.
#[derive(Debug, Clone, Deserialize)]
pub struct Subscription {
    pub endpoint: String,
    pub keys: Keys,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Keys {
    pub p256dh: String,
    pub auth: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Message<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub link: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Delivered,
    Gone,
    Rejected(u16),
}

/// The server's key pair and the contact a push service may reach its operator at.
pub struct Vapid {
    secret: SecretKey,
    sub: String,
}

impl Vapid {
    pub async fn load(state: &State) -> anyhow::Result<Self> {
        let sub = match (setup::email(state).await?, setup::hostname(state).await?) {
            (Some(email), _) => format!("mailto:{email}"),
            (None, Some(host)) => format!("https://{host}"),
            (None, None) => bail!("Push needs the panel hostname; finish setup first."),
        };
        Ok(Self {
            secret: secret(state).await?,
            sub,
        })
    }
}

pub async fn ensure_vapid(state: &State) -> anyhow::Result<()> {
    if state.get_setting(VAPID_SETTING).await?.is_some() {
        return Ok(());
    }
    let sealed = secrets::encrypt(
        &state.key,
        &URL_SAFE_NO_PAD.encode(random_secret().to_bytes()),
    );
    sqlx::query!(
        "INSERT INTO settings (key, value, updated_at) VALUES (?, ?, datetime('now'))
         ON CONFLICT(key) DO NOTHING",
        VAPID_SETTING,
        sealed
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn secret(state: &State) -> anyhow::Result<SecretKey> {
    ensure_vapid(state).await?;
    let stored = state
        .get_setting(VAPID_SETTING)
        .await?
        .context("the VAPID key vanished as it was created")?;
    vapid::secret_from_b64(&secrets::decrypt(&state.key, &stored)?)
}

/// The uncompressed point, base64url, as `applicationServerKey` wants it.
pub async fn public_key(state: &State) -> anyhow::Result<String> {
    Ok(vapid::public_key_b64(&secret(state).await?))
}

#[derive(Debug, thiserror::Error)]
pub enum InvalidSubscription {
    #[error("The endpoint is not a push service URL.")]
    Endpoint,
    #[error("The subscription keys are not a browser's push keys.")]
    Keys,
}

fn validate(sub: &Subscription) -> Result<(), InvalidSubscription> {
    let url = reqwest::Url::parse(&sub.endpoint).map_err(|_| InvalidSubscription::Endpoint)?;
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
        return Err(InvalidSubscription::Endpoint);
    }
    let point = URL_SAFE_NO_PAD
        .decode(sub.keys.p256dh.trim_end_matches('='))
        .ok()
        .filter(|p| p256::PublicKey::from_sec1_bytes(p).is_ok());
    let auth = URL_SAFE_NO_PAD
        .decode(sub.keys.auth.trim_end_matches('='))
        .ok()
        .filter(|a| a.len() == 16);
    if point.is_none() || auth.is_none() {
        return Err(InvalidSubscription::Keys);
    }
    Ok(())
}

/// Re-registering an endpoint moves it to `user_id` and takes the new keys.
pub async fn register(state: &State, user_id: &str, sub: &Subscription) -> anyhow::Result<Device> {
    validate(sub)?;
    let id = uuid::Uuid::new_v4().to_string();
    let auth = secrets::encrypt(&state.key, &sub.keys.auth);
    sqlx::query!(
        "INSERT INTO push_devices (id, user_id, endpoint, p256dh, auth) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(endpoint) DO UPDATE SET
           user_id = excluded.user_id, p256dh = excluded.p256dh, auth = excluded.auth",
        id,
        user_id,
        sub.endpoint,
        sub.keys.p256dh,
        auth
    )
    .execute(&state.pool)
    .await?;
    fetch(state, Some(user_id), None)
        .await?
        .into_iter()
        .find(|d| d.endpoint == sub.endpoint)
        .context("the device vanished as it was registered")
}

pub async fn unregister(state: &State, user_id: &str, endpoint: &str) -> anyhow::Result<bool> {
    let done = sqlx::query!(
        "DELETE FROM push_devices WHERE user_id = ? AND endpoint = ?",
        user_id,
        endpoint
    )
    .execute(&state.pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

pub async fn list_for(state: &State, user_id: &str) -> anyhow::Result<Vec<Device>> {
    fetch(state, Some(user_id), None).await
}

/// Every device whose owner has not switched `pref` off.
async fn listening(state: &State, pref: &str) -> anyhow::Result<Vec<Device>> {
    fetch(state, None, Some(pref)).await
}

async fn fetch(
    state: &State,
    user_id: Option<&str>,
    pref: Option<&str>,
) -> anyhow::Result<Vec<Device>> {
    let rows = sqlx::query!(
        r#"SELECT d.id AS "id!", d.user_id, d.endpoint, d.p256dh, d.auth, d.created_at, d.last_ok_at
           FROM push_devices d
           WHERE (?1 IS NULL OR d.user_id = ?1)
             AND (?2 IS NULL OR NOT EXISTS (
               SELECT 1 FROM push_prefs p WHERE p.user_id = d.user_id AND p.kind = ?2 AND p.enabled = 0))
           ORDER BY d.created_at, d.id"#,
        user_id,
        pref
    )
    .fetch_all(&state.pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok(Device {
                auth: secrets::decrypt(&state.key, &r.auth)?,
                id: r.id,
                user_id: r.user_id,
                endpoint: r.endpoint,
                p256dh: r.p256dh,
                created_at: time::utc(r.created_at),
                last_ok_at: time::utc_opt(r.last_ok_at),
            })
        })
        .collect()
}

pub async fn get_prefs(
    state: &State,
    user_id: &str,
) -> anyhow::Result<BTreeMap<&'static str, bool>> {
    let mut prefs: BTreeMap<&'static str, bool> = PREFS.iter().map(|p| (*p, true)).collect();
    let rows = sqlx::query!(
        r#"SELECT kind, enabled AS "enabled: bool" FROM push_prefs WHERE user_id = ?"#,
        user_id
    )
    .fetch_all(&state.pool)
    .await?;
    for row in rows {
        if let Some(pref) = PREFS.iter().find(|p| **p == row.kind) {
            prefs.insert(pref, row.enabled);
        }
    }
    Ok(prefs)
}

pub async fn set_pref(
    state: &State,
    user_id: &str,
    kind: &str,
    enabled: bool,
) -> anyhow::Result<()> {
    if !PREFS.contains(&kind) {
        bail!("{kind} is not a notification kind");
    }
    sqlx::query!(
        "INSERT INTO push_prefs (user_id, kind, enabled) VALUES (?, ?, ?)
         ON CONFLICT(user_id, kind) DO UPDATE SET enabled = excluded.enabled",
        user_id,
        kind,
        enabled
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// Encrypts `message` for one device and posts it to the device's push service.
pub async fn send(
    http: &reqwest::Client,
    vapid: &Vapid,
    device: &Device,
    message: &Message<'_>,
) -> anyhow::Result<Outcome> {
    let payload = serde_json::to_vec(message)?;
    let body = encrypt::encrypt(&payload, &device.p256dh, &device.auth)?;
    let authorization = vapid::header(
        &device.endpoint,
        &vapid.secret,
        &vapid.sub,
        chrono::Utc::now().timestamp(),
    )?;
    let res = http
        .post(&device.endpoint)
        .timeout(SEND_TIMEOUT)
        .header("TTL", TTL_SECS)
        .header("Content-Encoding", "aes128gcm")
        .header("Content-Type", "application/octet-stream")
        .header("Urgency", "normal")
        .header("Authorization", authorization)
        .body(body)
        .send()
        .await
        .context("reaching the push service")?;
    Ok(match res.status().as_u16() {
        200..=299 => Outcome::Delivered,
        404 | 410 => Outcome::Gone,
        status => Outcome::Rejected(status),
    })
}

/// Sends, then keeps the device table honest: a gone subscription is dropped, a delivery stamped.
pub async fn deliver(
    state: &State,
    http: &reqwest::Client,
    vapid: &Vapid,
    device: &Device,
    message: &Message<'_>,
) -> anyhow::Result<Outcome> {
    let outcome = send(http, vapid, device, message).await?;
    match outcome {
        Outcome::Delivered => {
            sqlx::query!(
                "UPDATE push_devices SET last_ok_at = datetime('now') WHERE id = ?",
                device.id
            )
            .execute(&state.pool)
            .await?;
        }
        Outcome::Gone => {
            sqlx::query!("DELETE FROM push_devices WHERE id = ?", device.id)
                .execute(&state.pool)
                .await?;
        }
        Outcome::Rejected(_) => {}
    }
    Ok(outcome)
}

async fn fan_out(state: &State, http: &reqwest::Client, event: &Event) -> anyhow::Result<()> {
    let Some(kind) = Kind::parse(&event.kind) else {
        return Ok(());
    };
    let Some(pref) = kind.pref() else {
        return Ok(());
    };
    let devices = listening(state, pref).await?;
    if devices.is_empty() {
        return Ok(());
    }
    let vapid = Vapid::load(state).await?;
    let message = Message {
        title: kind.title(),
        body: &event.sentence,
        link: event.link.as_deref().unwrap_or("/"),
    };
    for device in &devices {
        match deliver(state, http, &vapid, device, &message).await {
            Ok(Outcome::Delivered) => {}
            Ok(Outcome::Gone) => {
                tracing::info!(device = %device.id, "push subscription gone; dropped")
            }
            Ok(Outcome::Rejected(status)) => {
                tracing::warn!(device = %device.id, status, "push service refused a notification")
            }
            Err(error) => tracing::warn!(device = %device.id, error = ?error, "push not sent"),
        }
    }
    Ok(())
}

/// Delivers every event it is handed to the devices that want its kind.
pub fn spawn_fanout(
    state: State,
    http: reqwest::Client,
) -> tokio::sync::mpsc::UnboundedSender<Event> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Event>();
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if let Err(error) = fan_out(&state, &http, &event).await {
                tracing::warn!(kind = %event.kind, error = ?error, "push fan-out failed");
            }
        }
    });
    tx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::state;
    use crate::users;

    fn subscription(endpoint: &str) -> Subscription {
        let key = random_secret();
        Subscription {
            endpoint: endpoint.into(),
            keys: Keys {
                p256dh: vapid::public_key_b64(&key),
                auth: URL_SAFE_NO_PAD.encode(rand::random::<[u8; 16]>()),
            },
        }
    }

    #[tokio::test]
    async fn the_vapid_key_is_made_once_sealed_and_stays_the_same() {
        let (_d, state) = state().await;
        let first = public_key(&state).await.unwrap();
        ensure_vapid(&state).await.unwrap();
        assert_eq!(public_key(&state).await.unwrap(), first);
        let stored = state.get_setting(VAPID_SETTING).await.unwrap().unwrap();
        assert!(secrets::is_encrypted(&stored));
    }

    #[tokio::test]
    async fn a_device_is_registered_once_per_endpoint_and_prefs_default_on() {
        let (_d, state) = state().await;
        let saeed = users::create(&state, "Saeed").await.unwrap();
        let other = users::create(&state, "Other").await.unwrap();
        let sub = subscription("https://fcm.googleapis.com/fcm/send/abc");

        let first = register(&state, &saeed.id, &sub).await.unwrap();
        let again = register(&state, &saeed.id, &sub).await.unwrap();
        assert_eq!(first.id, again.id);
        assert_eq!(again.auth, sub.keys.auth, "stored sealed, read back plain");
        assert_eq!(list_for(&state, &saeed.id).await.unwrap().len(), 1);
        assert!(list_for(&state, &other.id).await.unwrap().is_empty());

        let mut bad = subscription("https://fcm.googleapis.com/fcm/send/def");
        bad.keys.auth = "short".into();
        let err = register(&state, &saeed.id, &bad).await.unwrap_err();
        assert!(err.downcast_ref::<InvalidSubscription>().is_some());

        assert!(
            get_prefs(&state, &saeed.id)
                .await
                .unwrap()
                .values()
                .all(|on| *on)
        );
        set_pref(&state, &saeed.id, "deploy_live", false)
            .await
            .unwrap();
        assert!(!get_prefs(&state, &saeed.id).await.unwrap()["deploy_live"]);
        assert!(listening(&state, "deploy_live").await.unwrap().is_empty());
        assert_eq!(listening(&state, "broke").await.unwrap().len(), 1);
        assert!(
            set_pref(&state, &saeed.id, "role_kept", true)
                .await
                .is_err()
        );

        assert!(!unregister(&state, &other.id, &sub.endpoint).await.unwrap());
        assert!(unregister(&state, &saeed.id, &sub.endpoint).await.unwrap());
        assert!(list_for(&state, &saeed.id).await.unwrap().is_empty());
    }
}
