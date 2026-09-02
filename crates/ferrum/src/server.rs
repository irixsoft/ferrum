use crate::auth::webauthn::Challenges;
use axum::Router;
use axum::routing::get;
use ferrum_core::github::Api;
use ferrum_core::runtime::Mirrors;
use ferrum_core::runtime::toolchain::Store;
use ferrum_core::state::State;
use ferrum_platform::{Platform, Ubuntu};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tower_http::trace::TraceLayer;

pub use ferrum_core::LISTEN_ADDR;

/// Everything a test may want to stand a stub in for, exactly as `Directory::Custom` does for
/// Let's Encrypt.
#[derive(Clone)]
pub struct Deps {
    pub github: Api,
    pub platform: Arc<dyn Platform>,
    pub toolchains: Store,
    pub mirrors: Mirrors,
    pub codename: String,
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
}

impl AppState {
    pub fn new(db: State, deps: Deps) -> Self {
        Self {
            db,
            challenges: Challenges::default(),
            http: ferrum_core::http::client(),
            github: deps.github,
            platform: deps.platform,
            toolchains: deps.toolchains,
            mirrors: deps.mirrors,
            codename: deps.codename,
            postgres_install: Arc::default(),
        }
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
    let listener = tokio::net::TcpListener::bind(LISTEN_ADDR).await?;
    tracing::info!(addr = %LISTEN_ADDR, "listening");
    axum::serve(listener, app(state))
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
