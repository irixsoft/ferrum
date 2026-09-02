use crate::auth::webauthn::Challenges;
use axum::Router;
use axum::routing::get;
use ferrum_core::acme::Directory;
use ferrum_core::certs::Issuance;
use ferrum_core::deploy::{Ctx, Deployer};
use ferrum_core::dns::Lookup;
use ferrum_core::github::Api;
use ferrum_core::runtime::Mirrors;
use ferrum_core::runtime::toolchain::Store;
use ferrum_core::state::State;
use ferrum_platform::{Platform, Ubuntu};
use std::net::IpAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tower_http::trace::TraceLayer;

pub use ferrum_core::LISTEN_ADDR;

const ACME_DIRECTORY_SETTING: &str = "acme.directory";

/// Everything a test may want to stand a stub in for, exactly as `Directory::Custom` does for
/// Let's Encrypt.
#[derive(Clone)]
pub struct Deps {
    pub github: Api,
    pub platform: Arc<dyn Platform>,
    pub toolchains: Store,
    pub mirrors: Mirrors,
    pub codename: String,
    pub directory: Directory,
    pub lookup: Lookup,
    pub public_ip: Option<IpAddr>,
}

impl Default for Deps {
    fn default() -> Self {
        Self {
            github: Api::default(),
            platform: Arc::new(Ubuntu),
            toolchains: Store::default(),
            mirrors: Mirrors::default(),
            codename: ferrum_platform::detect()
                .map(|host| host.codename)
                .unwrap_or_default(),
            directory: Directory::LetsEncrypt,
            lookup: Lookup::Public,
            public_ip: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Install {
    #[default]
    Idle,
    Running,
    Failed(String),
}

#[derive(Clone)]
pub struct AppState {
    pub db: State,
    pub challenges: Challenges,
    pub http: reqwest::Client,
    pub github: Api,
    pub platform: Arc<dyn Platform>,
    pub toolchains: Store,
    pub mirrors: Mirrors,
    pub codename: String,
    pub postgres_install: Arc<Mutex<Install>>,
    pub deployer: Deployer,
    pub certs: Issuance,
}

impl AppState {
    pub fn new(db: State, deps: Deps) -> Self {
        let http = ferrum_core::http::client();
        let ctx = Ctx::new(
            db.clone(),
            deps.platform.clone(),
            deps.github.clone(),
            http.clone(),
            deps.toolchains.clone(),
        );
        Self {
            db,
            challenges: Challenges::default(),
            http,
            github: deps.github,
            platform: deps.platform,
            toolchains: deps.toolchains,
            mirrors: deps.mirrors,
            codename: deps.codename,
            postgres_install: Arc::default(),
            deployer: Deployer::start(ctx),
            certs: Issuance::new(deps.directory, deps.lookup, deps.public_ip),
        }
    }

    /// Certificates take a minute and a request must not wait for one.
    pub fn issue_certificates_later(&self, app: ferrum_core::apps::App) {
        if app.domains.is_empty() {
            return;
        }
        let state = self.clone();
        tokio::spawn(async move {
            if let Err(e) = ferrum_core::certs::issue_for(
                &state.db,
                state.platform.as_ref(),
                &state.certs,
                &app,
            )
            .await
            {
                tracing::warn!(app = %app.slug, error = ?e, "certificate issuance failed");
            }
        });
    }
}

pub fn app(db: State) -> Router {
    router(AppState::new(db, Deps::default()))
}

pub fn app_with_github(db: State, github: Api) -> Router {
    app_with(
        db,
        Deps {
            github,
            ..Deps::default()
        },
    )
}

pub fn app_with(db: State, deps: Deps) -> Router {
    router(AppState::new(db, deps))
}

pub async fn set_acme_directory(state: &State, staging: bool) -> anyhow::Result<()> {
    let name = if staging { "staging" } else { "production" };
    state.set_setting(ACME_DIRECTORY_SETTING, name).await
}

pub async fn acme_directory(state: &State) -> anyhow::Result<Directory> {
    Ok(
        match state.get_setting(ACME_DIRECTORY_SETTING).await?.as_deref() {
            Some("staging") => Directory::Staging,
            _ => Directory::LetsEncrypt,
        },
    )
}

fn router(state: AppState) -> Router {
    let public = Router::new()
        .route("/api/health", get(crate::routes::health::get))
        .route("/api/version", get(crate::routes::version::get))
        .merge(crate::routes::auth::router())
        .merge(crate::routes::github::public_router())
        .merge(crate::routes::webhook::router());

    let protected = Router::new()
        .route("/api/me", get(crate::routes::me::get))
        .merge(crate::routes::users::router())
        .merge(crate::routes::sessions::router())
        .merge(crate::routes::tokens::router())
        .merge(crate::routes::github::router())
        .merge(crate::routes::apps::router())
        .merge(crate::routes::runtimes::router())
        .merge(crate::routes::databases::router())
        .merge(crate::routes::deploys::router())
        .merge(crate::routes::host::router())
        .merge(crate::routes::logs::router())
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::require_caller,
        ));

    public
        .merge(protected)
        .merge(crate::panel::router())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

pub async fn serve(data_dir: &Path) -> anyhow::Result<()> {
    let state = State::open(data_dir).await?;
    let deps = Deps {
        directory: acme_directory(&state).await?,
        ..Deps::default()
    };
    let app_state = AppState::new(state.clone(), deps);
    ferrum_core::certs::spawn_sweeper(
        state.clone(),
        app_state.platform.clone(),
        app_state.certs.clone(),
    );
    ferrum_core::metrics::spawn_sampler(state, app_state.platform.clone());
    let listener = tokio::net::TcpListener::bind(LISTEN_ADDR).await?;
    tracing::info!(addr = %LISTEN_ADDR, "listening");
    axum::serve(listener, router(app_state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    let term = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}
