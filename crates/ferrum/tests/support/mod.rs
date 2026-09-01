#![allow(dead_code)]

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ferrum_core::github::Api;
use ferrum_core::state::State;
use ferrum_core::{enrollment, setup, users};
use serde_json::Value;
use tower::ServiceExt;
use webauthn_authenticator_rs::AuthenticatorBackend;
use webauthn_authenticator_rs::softpasskey::SoftPasskey;
use webauthn_rs::prelude::Url;
use webauthn_rs_proto::{
    AllowCredentials, PublicKeyCredential, PublicKeyCredentialCreationOptions,
    PublicKeyCredentialRequestOptions, RegisterPublicKeyCredential,
};

pub const HOSTNAME: &str = "panel.example.com";
pub const USER_AGENT: &str = "ferrum-tests";
const TIMEOUT_MS: u32 = 60_000;

pub struct Harness {
    pub app: Router,
    pub db: State,
    _dir: tempfile::TempDir,
}

pub struct Res {
    pub status: StatusCode,
    pub json: Value,
    pub text: String,
    pub headers: axum::http::HeaderMap,
    pub set_cookie: Vec<String>,
}

impl Res {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    pub fn session_cookie(&self) -> Option<String> {
        self.set_cookie
            .iter()
            .find_map(|c| c.strip_prefix("ferrum_session="))
            .map(|rest| rest.split(';').next().unwrap_or_default().to_string())
            .filter(|v| !v.is_empty())
    }
}

/// Nothing listens here, so a test that reaches for github.com fails at once instead of
/// silently making a real request.
pub const NO_GITHUB: &str = "http://127.0.0.1:1";

pub async fn harness() -> Harness {
    harness_with_github(NO_GITHUB).await
}

pub async fn harness_with_github(base: &str) -> Harness {
    let h = harness_without_hostname(base).await;
    setup::set_hostname(&h.db, HOSTNAME).await.unwrap();
    h
}

pub async fn harness_without_hostname(base: &str) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let db = State::open(dir.path()).await.unwrap();
    Harness {
        app: ferrum::server::app_with_github(db.clone(), Api::at(base)),
        db,
        _dir: dir,
    }
}

pub fn soft_passkey() -> SoftPasskey {
    SoftPasskey::new(true)
}

pub async fn signed_in() -> (Harness, String) {
    let h = harness().await;
    let link = h.enrollment("Saeed").await;
    let mut key = soft_passkey();
    let cookie = h.register(&mut key, &link).await.session_cookie().unwrap();
    (h, cookie)
}

pub const TEST_PEM: &str =
    "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKC\n-----END RSA PRIVATE KEY-----\n";
pub const WEBHOOK_SECRET: &str = "whsec_test";

/// `SoftPasskey` refuses `requireResidentKey`, and never returns a `userHandle`, so it cannot
/// act as a discoverable authenticator. These two adjustments stand in for the platform
/// authenticator a real user has. Everything the server does — the challenge it sends, the
/// signature it verifies, the identity it derives from the handle — is exercised unchanged.
/// That the server *asks* for a discoverable credential is asserted separately, in the unit
/// tests for `auth::webauthn`.
fn relax_for_soft_authenticator(options: &mut PublicKeyCredentialCreationOptions) {
    if let Some(selection) = options.authenticator_selection.as_mut() {
        selection.require_resident_key = false;
        selection.resident_key = None;
    }
}

impl Harness {
    pub fn origin(&self) -> Url {
        Url::parse(&format!("https://{HOSTNAME}")).unwrap()
    }

    pub async fn enrollment(&self, name: &str) -> String {
        let user = users::create(&self.db, name).await.unwrap();
        enrollment::issue(&self.db, &user.id).await.unwrap()
    }

    pub async fn send(&self, mut req: Request<Body>) -> Res {
        req.headers_mut()
            .entry(header::USER_AGENT)
            .or_insert(USER_AGENT.parse().unwrap());
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let set_cookie = headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .map(str::to_string)
            .collect();

        let bytes = axum::body::to_bytes(res.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

        Res {
            status,
            json,
            text: String::from_utf8_lossy(&bytes).into_owned(),
            headers,
            set_cookie,
        }
    }

    pub async fn get(&self, uri: &str) -> Res {
        self.send(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
    }

    pub async fn get_with_cookie(&self, uri: &str, cookie: &str) -> Res {
        self.send(
            Request::builder()
                .uri(uri)
                .header(header::COOKIE, format!("ferrum_session={cookie}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    pub async fn machine_token(&self, read_only: bool) -> String {
        ferrum_core::tokens::mint(&self.db, "agent", read_only)
            .await
            .unwrap()
            .secret
    }

    pub async fn connect_github(&self) -> ferrum_core::github::Connection {
        ferrum_core::github::save(
            &self.db,
            ferrum_core::github::NewConnection {
                app_id: 12345,
                app_slug: "ferrum-panel-example".into(),
                app_name: "ferrum-panel-example".into(),
                account: "irixsoft".into(),
                private_key: TEST_PEM.into(),
                webhook_secret: WEBHOOK_SECRET.into(),
                client_id: "Iv1.abc".into(),
                client_secret: "cs_abc".into(),
            },
        )
        .await
        .unwrap()
    }

    pub async fn connect_state(&self, cookie: &str) -> String {
        self.post_with_cookie("/api/github/connect", "", cookie)
            .await
            .json["state"]
            .as_str()
            .expect("connect returns a state")
            .to_string()
    }

    pub async fn post_with_bearer(&self, uri: &str, body: &str, token: &str) -> Res {
        self.send(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
    }

    pub async fn delete_with_bearer(&self, uri: &str, token: &str) -> Res {
        self.send(
            Request::builder()
                .method("DELETE")
                .uri(uri)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    pub async fn get_with_bearer(&self, uri: &str, token: &str) -> Res {
        self.send(
            Request::builder()
                .uri(uri)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    pub async fn post(&self, uri: &str, body: &str) -> Res {
        self.send(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
    }

    pub async fn post_with_cookie(&self, uri: &str, body: &str, cookie: &str) -> Res {
        self.send(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, format!("ferrum_session={cookie}"))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
    }

    pub async fn delete_with_cookie(&self, uri: &str, cookie: &str) -> Res {
        self.send(
            Request::builder()
                .method("DELETE")
                .uri(uri)
                .header(header::COOKIE, format!("ferrum_session={cookie}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    pub async fn try_register_begin(
        &self,
        link: &str,
    ) -> Result<Begun<PublicKeyCredentialCreationOptions>, Box<Res>> {
        let res = self
            .post(
                "/api/auth/register/begin",
                &format!(r#"{{"enrollment":"{link}"}}"#),
            )
            .await;
        if res.status != StatusCode::OK {
            return Err(Box::new(res));
        }

        let mut options: PublicKeyCredentialCreationOptions =
            serde_json::from_value(res.json["publicKey"].clone()).unwrap();
        relax_for_soft_authenticator(&mut options);

        Ok(Begun {
            id: res.json["id"].as_str().unwrap().to_string(),
            options,
        })
    }

    pub async fn register_begin(&self, link: &str) -> Begun<PublicKeyCredentialCreationOptions> {
        self.try_register_begin(link)
            .await
            .unwrap_or_else(|res| panic!("register/begin refused: {} {}", res.status, res.json))
    }

    pub async fn register(&self, key: &mut SoftPasskey, link: &str) -> Res {
        let begun = match self.try_register_begin(link).await {
            Ok(begun) => begun,
            Err(res) => return *res,
        };
        let credential = key
            .register(&self.origin(), &begun)
            .expect("the soft passkey creates a credential");
        let body = serde_json::json!({
            "id": begun.id,
            "enrollment": link,
            "credential": credential,
        });
        self.post("/api/auth/register/finish", &body.to_string())
            .await
    }

    /// Stands in for the browser: the server sends an empty `allowCredentials`, and a real
    /// platform authenticator answers it from its own resident credentials, returning the
    /// `userHandle`. Neither soft authenticator in `webauthn-authenticator-rs` is discoverable,
    /// so the discovery step is performed here while the assertion itself is really signed.
    pub async fn assertion(&self, key: &mut SoftPasskey) -> Assertion {
        let res = self.post("/api/auth/login/begin", "{}").await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.json);

        let mut options: PublicKeyCredentialRequestOptions =
            serde_json::from_value(res.json["publicKey"].clone()).unwrap();
        assert!(
            options.allow_credentials.is_empty(),
            "the server must not name the credential it expects"
        );

        let (user, stored) = self.only_credential().await;
        options.allow_credentials = vec![AllowCredentials {
            type_: "public-key".to_string(),
            id: URL_SAFE_NO_PAD.decode(&stored).unwrap(),
            transports: None,
        }];

        let begun = Begun {
            id: res.json["id"].as_str().unwrap().to_string(),
            options,
        };
        let mut credential = key.authenticate(&self.origin(), &begun);
        credential.response.user_handle = Some(handle_bytes(&user));

        Assertion {
            id: begun.id,
            credential,
        }
    }

    pub async fn login_with(&self, assertion: Assertion) -> Res {
        let body = serde_json::json!({
            "id": assertion.id,
            "credential": assertion.credential,
        });
        self.post("/api/auth/login/finish", &body.to_string()).await
    }

    pub async fn login(&self, key: &mut SoftPasskey) -> Res {
        let assertion = self.assertion(key).await;
        self.login_with(assertion).await
    }

    async fn only_credential(&self) -> (String, String) {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT users.handle, credentials.id FROM credentials JOIN users ON users.id = credentials.user_id")
                .fetch_all(&self.db.pool)
                .await
                .unwrap();
        assert_eq!(
            rows.len(),
            1,
            "this helper assumes a single enrolled passkey"
        );
        rows.into_iter().next().unwrap()
    }
}

pub struct Assertion {
    pub id: String,
    pub credential: PublicKeyCredential,
}

pub fn handle_bytes(handle: &str) -> Vec<u8> {
    uuid::Uuid::parse_str(handle).unwrap().as_bytes().to_vec()
}

pub struct Begun<T> {
    pub id: String,
    pub options: T,
}

pub trait Ceremony {
    fn register(
        &mut self,
        origin: &Url,
        begun: &Begun<PublicKeyCredentialCreationOptions>,
    ) -> Result<RegisterPublicKeyCredential, String>;

    fn authenticate(
        &mut self,
        origin: &Url,
        begun: &Begun<PublicKeyCredentialRequestOptions>,
    ) -> PublicKeyCredential;
}

impl Ceremony for SoftPasskey {
    fn register(
        &mut self,
        origin: &Url,
        begun: &Begun<PublicKeyCredentialCreationOptions>,
    ) -> Result<RegisterPublicKeyCredential, String> {
        self.perform_register(origin.clone(), begun.options.clone(), TIMEOUT_MS)
            .map_err(|e| format!("{e:?}"))
    }

    fn authenticate(
        &mut self,
        origin: &Url,
        begun: &Begun<PublicKeyCredentialRequestOptions>,
    ) -> PublicKeyCredential {
        self.perform_auth(origin.clone(), begun.options.clone(), TIMEOUT_MS)
            .expect("the soft passkey signs the assertion")
    }
}
