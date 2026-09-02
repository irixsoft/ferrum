use crate::state::State;
use aes_gcm::Aes256Gcm;
use aes_gcm::aead::{Aead, KeyInit};
use anyhow::{Context, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use zeroize::Zeroizing;

pub const KEY_FILE: &str = "secret.key";
pub const MISSING_KEY: &str = "secret.key is missing; restore it from your backup";
const PREFIX: &str = "v1:";
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;

pub struct Key(Zeroizing<[u8; KEY_LEN]>);

impl Key {
    pub fn open(data_dir: &Path) -> anyhow::Result<Option<Self>> {
        let path = data_dir.join(KEY_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let bytes = STANDARD
            .decode(text.trim())
            .ok()
            .and_then(|b| <[u8; KEY_LEN]>::try_from(b).ok())
            .with_context(|| format!("{} is not a Ferrum key", path.display()))?;
        Ok(Some(Self(Zeroizing::new(bytes))))
    }

    pub fn create(data_dir: &Path) -> anyhow::Result<Self> {
        let key = Self::random();
        let path = data_dir.join(KEY_FILE);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .with_context(|| format!("creating {}", path.display()))?;
        std::io::Write::write_all(
            &mut file,
            format!("{}\n", STANDARD.encode(*key.0)).as_bytes(),
        )?;
        Ok(key)
    }

    pub fn random() -> Self {
        Self(Zeroizing::new(rand::random()))
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new_from_slice(&self.0[..]).expect("a 32-byte key")
    }
}

pub fn is_encrypted(stored: &str) -> bool {
    stored.starts_with(PREFIX)
}

pub fn encrypt(key: &Key, plain: &str) -> String {
    let nonce: [u8; NONCE_LEN] = rand::random();
    let sealed = key
        .cipher()
        .encrypt(aes_gcm::Nonce::from_slice(&nonce), plain.as_bytes())
        .expect("sealing an in-memory buffer");
    format!(
        "{PREFIX}{}:{}",
        STANDARD.encode(nonce),
        STANDARD.encode(sealed)
    )
}

/// A value without the prefix is returned as it is, so a half-migrated database still reads.
pub fn decrypt(key: &Key, stored: &str) -> anyhow::Result<String> {
    let Some(rest) = stored.strip_prefix(PREFIX) else {
        return Ok(stored.to_string());
    };
    let (nonce, sealed) = rest
        .split_once(':')
        .context("a stored secret is malformed")?;
    let nonce = STANDARD.decode(nonce)?;
    let sealed = STANDARD.decode(sealed)?;
    if nonce.len() != NONCE_LEN {
        bail!("a stored secret is malformed");
    }
    let plain = key
        .cipher()
        .decrypt(aes_gcm::Nonce::from_slice(&nonce), sealed.as_slice())
        .map_err(|_| anyhow::anyhow!("a stored secret does not decrypt with {KEY_FILE}"))?;
    Ok(String::from_utf8(plain)?)
}

pub async fn any_encrypted(pool: &sqlx::SqlitePool) -> anyhow::Result<bool> {
    let row = sqlx::query!(
        r#"SELECT count(*) AS "n!: i64" FROM (
             SELECT value AS v FROM app_env
             UNION ALL SELECT password FROM databases
             UNION ALL SELECT password FROM redis_instances
             UNION ALL SELECT private_key FROM github_app
           ) WHERE substr(v, 1, 3) = 'v1:'"#
    )
    .fetch_one(pool)
    .await?;
    Ok(row.n > 0)
}

/// Encrypts every plaintext secret still in the database; a second run finds nothing to do.
pub async fn migrate(state: &State) -> anyhow::Result<u64> {
    let key = &state.key;
    let mut done = 0;
    for row in sqlx::query!(
        r#"SELECT app_id AS "app_id!", key AS "key!", value AS "value!"
           FROM app_env WHERE substr(value, 1, 3) <> 'v1:'"#
    )
    .fetch_all(&state.pool)
    .await?
    {
        let sealed = encrypt(key, &row.value);
        sqlx::query!(
            "UPDATE app_env SET value = ? WHERE app_id = ? AND key = ?",
            sealed,
            row.app_id,
            row.key
        )
        .execute(&state.pool)
        .await?;
        done += 1;
    }
    for row in sqlx::query!(
        r#"SELECT id AS "id!", password AS "password!" FROM databases
           WHERE substr(password, 1, 3) <> 'v1:'"#
    )
    .fetch_all(&state.pool)
    .await?
    {
        let sealed = encrypt(key, &row.password);
        sqlx::query!(
            "UPDATE databases SET password = ? WHERE id = ?",
            sealed,
            row.id
        )
        .execute(&state.pool)
        .await?;
        done += 1;
    }
    for row in sqlx::query!(
        r#"SELECT app_id AS "app_id!", password AS "password!" FROM redis_instances
           WHERE substr(password, 1, 3) <> 'v1:'"#
    )
    .fetch_all(&state.pool)
    .await?
    {
        let sealed = encrypt(key, &row.password);
        sqlx::query!(
            "UPDATE redis_instances SET password = ? WHERE app_id = ?",
            sealed,
            row.app_id
        )
        .execute(&state.pool)
        .await?;
        done += 1;
    }
    if let Some(row) = sqlx::query!(
        r#"SELECT private_key AS "private_key!", webhook_secret AS "webhook_secret!",
                  client_secret AS "client_secret!"
           FROM github_app WHERE id = 1 AND substr(private_key, 1, 3) <> 'v1:'"#
    )
    .fetch_optional(&state.pool)
    .await?
    {
        let private_key = encrypt(key, &row.private_key);
        let webhook_secret = encrypt(key, &row.webhook_secret);
        let client_secret = encrypt(key, &row.client_secret);
        sqlx::query!(
            "UPDATE github_app SET private_key = ?, webhook_secret = ?, client_secret = ? WHERE id = 1",
            private_key,
            webhook_secret,
            client_secret
        )
        .execute(&state.pool)
        .await?;
        done += 1;
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::{new_app, state};
    use crate::apps::{self, env};
    use crate::{github, postgres, redis};
    use ferrum_platform::FakePlatform;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_secret_round_trips_and_a_tampered_one_does_not() {
        let key = Key::random();
        let stored = encrypt(&key, "hunter2");
        assert!(is_encrypted(&stored));
        assert!(stored.starts_with("v1:"));
        assert_ne!(encrypt(&key, "hunter2"), stored, "a fresh nonce every time");
        assert_eq!(decrypt(&key, &stored).unwrap(), "hunter2");
        assert_eq!(decrypt(&key, "plain").unwrap(), "plain");
        let mut bytes = stored.into_bytes();
        let last = bytes.len() - 1;
        bytes[last] = if bytes[last] == b'A' { b'B' } else { b'A' };
        assert!(decrypt(&key, &String::from_utf8(bytes).unwrap()).is_err());
        assert!(decrypt(&Key::random(), &encrypt(&key, "x")).is_err());
        assert!(decrypt(&key, "v1:garbage").is_err());
    }

    #[test]
    fn the_key_file_is_created_0600_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Key::open(dir.path()).unwrap().is_none());
        let key = Key::create(dir.path()).unwrap();
        let mode = std::fs::metadata(dir.path().join(KEY_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let again = Key::open(dir.path()).unwrap().unwrap();
        assert_eq!(
            decrypt(&again, &encrypt(&key, "same key")).unwrap(),
            "same key"
        );
        assert!(Key::create(dir.path()).is_err(), "never overwrite a key");
        std::fs::write(dir.path().join(KEY_FILE), "not base64!").unwrap();
        assert!(Key::open(dir.path()).is_err());
    }

    #[tokio::test]
    async fn every_secret_column_is_stored_sealed_and_read_back_in_the_clear() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        env::set(&state, &app.id, "SECRET", "hunter2")
            .await
            .unwrap();
        postgres::create(&state, &p, postgres::tests::new("ledger_prod"))
            .await
            .unwrap();
        postgres::link(&state, &app.id, "ledger_prod")
            .await
            .unwrap();
        redis::request(&state, &p, &app, 64).await.unwrap();
        github::save(&state, github::tests::sample()).await.unwrap();

        let raw: Vec<String> = sqlx::query_scalar(
            "SELECT value FROM app_env UNION ALL SELECT password FROM databases
             UNION ALL SELECT password FROM redis_instances
             UNION ALL SELECT private_key FROM github_app
             UNION ALL SELECT webhook_secret FROM github_app
             UNION ALL SELECT client_secret FROM github_app",
        )
        .fetch_all(&state.pool)
        .await
        .unwrap();
        assert_eq!(raw.len(), 6);
        assert!(raw.iter().all(|v| is_encrypted(v)), "{raw:?}");
        assert!(
            raw.iter()
                .all(|v| !v.contains("hunter2") && !v.contains("PRIVATE"))
        );

        assert_eq!(
            env::all(&state, &app.id).await.unwrap(),
            vec![("SECRET".to_string(), "hunter2".to_string())]
        );
        let urls = postgres::urls_for(&state, &app.id).await.unwrap();
        assert!(!urls[0].1.contains("v1:"), "{}", urls[0].1);
        let redis_url = redis::url_for(&state, &app.id).await.unwrap().unwrap();
        assert!(!redis_url.contains("v1:"), "{redis_url}");
        assert_eq!(
            github::private_key(&state).await.unwrap().as_deref(),
            Some(github::tests::TEST_PEM)
        );
        assert_eq!(
            github::webhook_secret(&state).await.unwrap().as_deref(),
            Some("whsec_test")
        );
        assert_eq!(migrate(&state).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn plaintext_rows_are_migrated_once_and_a_missing_key_refuses_to_start() {
        let dir = tempfile::tempdir().unwrap();
        let state = State::open(dir.path()).await.unwrap();
        let app = apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        sqlx::query("INSERT INTO app_env (app_id, key, value) VALUES (?, 'OLD', 'plain')")
            .bind(&app.id)
            .execute(&state.pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO databases (id, name, role, password, connection_limit)
             VALUES ('d1', 'legacy', 'legacy', 'pw', 10)",
        )
        .execute(&state.pool)
        .await
        .unwrap();
        assert!(!any_encrypted(&state.pool).await.unwrap());
        assert_eq!(migrate(&state).await.unwrap(), 2);
        assert_eq!(migrate(&state).await.unwrap(), 0);
        assert!(any_encrypted(&state.pool).await.unwrap());
        assert_eq!(
            env::all(&state, &app.id).await.unwrap(),
            vec![("OLD".to_string(), "plain".to_string())]
        );
        drop(state);

        let reopened = State::open(dir.path()).await.unwrap();
        assert_eq!(
            env::all(&reopened, &app.id).await.unwrap()[0].1,
            "plain",
            "the same key comes back from disk"
        );
        drop(reopened);

        std::fs::remove_file(dir.path().join(KEY_FILE)).unwrap();
        let refused = State::open(dir.path()).await;
        let message = format!("{:#}", refused.err().expect("no key, sealed rows"));
        assert!(message.contains(MISSING_KEY), "{message}");
    }
}
