use anyhow::{Context, anyhow, bail};
use ferrum_core::users::User;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;
use webauthn_rs::prelude::*;
use webauthn_rs_proto::ResidentKeyRequirement;

pub const CHALLENGE_TTL: Duration = Duration::from_secs(300);
const RP_NAME: &str = "Ferrum";

pub fn instance(hostname: &str) -> anyhow::Result<Webauthn> {
    let host = crate::setup::prompt::validate_hostname(hostname).map_err(|e| anyhow!("{e}"))?;
    let origin =
        Url::parse(&format!("https://{host}")).context("building the relying party origin")?;
    WebauthnBuilder::new(&host, &origin)
        .and_then(|b| b.rp_name(RP_NAME).build())
        .map_err(|e| anyhow!("configuring webauthn for {host}: {e}"))
}

pub fn start_registration(
    webauthn: &Webauthn,
    user: &User,
    already_held: Vec<CredentialID>,
) -> anyhow::Result<(CreationChallengeResponse, PasskeyRegistration)> {
    let handle = user
        .handle_uuid()
        .context("this account's handle is not a uuid, so no passkey can be created for it")?;
    let exclude = (!already_held.is_empty()).then_some(already_held);

    let (mut challenge, registration) =
        webauthn.start_passkey_registration(handle, &user.name, &user.name, exclude)?;
    require_discoverable(&mut challenge);
    Ok((challenge, registration))
}

fn require_discoverable(challenge: &mut CreationChallengeResponse) {
    if let Some(selection) = challenge.public_key.authenticator_selection.as_mut() {
        selection.resident_key = Some(ResidentKeyRequirement::Required);
        selection.require_resident_key = true;
    }
}

pub fn assert_discoverable(credential: &RegisterPublicKeyCredential) -> anyhow::Result<()> {
    let rk = credential.extensions.cred_props.as_ref().and_then(|p| p.rk);
    if rk == Some(false) {
        bail!(
            "That authenticator stored a passkey it cannot discover on its own, so signing in without a username would not work. Try a device passkey, or a security key with room for a resident credential."
        );
    }
    Ok(())
}

pub fn start_login(
    webauthn: &Webauthn,
) -> anyhow::Result<(RequestChallengeResponse, DiscoverableAuthentication)> {
    let (mut challenge, authentication) = webauthn.start_discoverable_authentication()?;
    challenge.mediation = None;
    Ok((challenge, authentication))
}

pub enum Pending {
    Register {
        user_id: String,
        state: Box<PasskeyRegistration>,
    },
    Login(Box<DiscoverableAuthentication>),
}

#[derive(Clone, Default)]
pub struct Challenges(Arc<Mutex<HashMap<String, (Instant, Pending)>>>);

impl Challenges {
    pub fn put(&self, pending: Pending) -> String {
        let id = Uuid::new_v4().to_string();
        let mut held = self.0.lock().expect("challenge store poisoned");
        held.retain(|_, (issued, _)| issued.elapsed() < CHALLENGE_TTL);
        held.insert(id.clone(), (Instant::now(), pending));
        id
    }

    pub fn take(&self, id: &str) -> Option<Pending> {
        let mut held = self.0.lock().expect("challenge store poisoned");
        held.remove(id)
            .filter(|(issued, _)| issued.elapsed() < CHALLENGE_TTL)
            .map(|(_, pending)| pending)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rp_id_is_the_configured_hostname() {
        let w = instance("panel.example.com").unwrap();
        assert_eq!(
            w.get_allowed_origins()[0].as_str(),
            "https://panel.example.com/"
        );
    }

    #[test]
    fn an_ip_address_is_refused_as_an_rp_id() {
        assert!(instance("203.0.113.10").is_err());
        assert!(instance("::1").is_err());
    }

    #[test]
    fn a_url_is_refused_as_an_rp_id() {
        assert!(instance("https://panel.example.com").is_err());
        assert!(instance("panel.example.com/login").is_err());
    }

    #[test]
    fn a_challenge_can_only_be_taken_once() {
        let store = Challenges::default();
        let id = store.put(Pending::Register {
            user_id: "u1".into(),
            state: Box::new(unreachable_registration()),
        });
        assert!(store.take(&id).is_some());
        assert!(store.take(&id).is_none());
        assert!(store.take("never-issued").is_none());
    }

    #[test]
    fn registration_asks_for_a_discoverable_credential() {
        let w = instance("panel.example.com").unwrap();
        let user = User {
            id: "u1".into(),
            handle: Uuid::new_v4().to_string(),
            name: "Saeed".into(),
            created_at: String::new(),
        };
        let (challenge, _) = start_registration(&w, &user, Vec::new()).unwrap();
        let json = serde_json::to_value(&challenge).unwrap();

        assert_eq!(
            json["publicKey"]["authenticatorSelection"]["residentKey"], "required",
            "a non-discoverable credential would put a username field back on the login page"
        );
        assert_eq!(
            json["publicKey"]["authenticatorSelection"]["requireResidentKey"],
            true
        );
    }

    #[test]
    fn registration_excludes_passkeys_the_account_already_holds() {
        let w = instance("panel.example.com").unwrap();
        let user = User {
            id: "u1".into(),
            handle: Uuid::new_v4().to_string(),
            name: "Saeed".into(),
            created_at: String::new(),
        };

        let (bare, _) = start_registration(&w, &user, Vec::new()).unwrap();
        assert!(
            bare.public_key.exclude_credentials.is_none(),
            "a first passkey has nothing to exclude"
        );

        let (again, _) = start_registration(&w, &user, vec![vec![1, 2, 3]]).unwrap();
        let excluded = again.public_key.exclude_credentials.unwrap();
        assert_eq!(excluded.len(), 1);
        assert_eq!(
            excluded[0].id,
            vec![1, 2, 3],
            "an authenticator must be told not to enrol itself twice"
        );
    }

    #[test]
    fn login_sends_no_allow_credentials_and_no_mediation() {
        let w = instance("panel.example.com").unwrap();
        let (challenge, _) = start_login(&w).unwrap();
        let json = serde_json::to_value(&challenge).unwrap();

        let allow = &json["publicKey"]["allowCredentials"];
        assert!(
            allow.is_null() || allow.as_array().is_some_and(|a| a.is_empty()),
            "allowCredentials must be empty so the browser picker supplies the identity: {json}"
        );
        assert!(
            json.get("mediation").is_none_or(|m| m.is_null()),
            "conditional mediation needs an autofill field, which the login page does not have"
        );
    }

    #[test]
    fn a_user_without_a_uuid_handle_cannot_register() {
        let w = instance("panel.example.com").unwrap();
        let user = User {
            id: "u1".into(),
            handle: "not-a-uuid".into(),
            name: "Saeed".into(),
            created_at: String::new(),
        };
        assert!(start_registration(&w, &user, Vec::new()).is_err());
    }

    fn unreachable_registration() -> PasskeyRegistration {
        let w = instance("panel.example.com").unwrap();
        let user = User {
            id: "u1".into(),
            handle: Uuid::new_v4().to_string(),
            name: "Saeed".into(),
            created_at: String::new(),
        };
        start_registration(&w, &user, Vec::new()).unwrap().1
    }

    #[test]
    fn expired_challenges_are_swept_on_insert() {
        let store = Challenges::default();
        store.put(Pending::Login(Box::new(
            start_login(&instance("panel.example.com").unwrap())
                .unwrap()
                .1,
        )));
        assert_eq!(store.len(), 1);
    }
}
