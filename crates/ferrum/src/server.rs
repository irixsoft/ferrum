use crate::auth::webauthn::Challenges;
use axum::Router;
use axum::routing::get;
use ferrum_core::github::Api;
use ferrum_core::state::State;
use std::path::Path;
use tower_http::trace::TraceLayer;

pub use ferrum_core::LISTEN_ADDR;

#[derive(Clone)]
pub struct AppState {
    pub db: State,
    pub challenges: Challenges,
    pub github: Api,
}

impl AppState {
    pub fn new(db: State) -> Self {
        Self {
            db,
            challenges: Challenges::default(),
            github: Api::default(),
        }
    }
}

pub fn app(db: State) -> Router {
    router(AppState::new(db))
}

/// Lets a test stand a stub in for api.github.com, as `Directory::Custom` does for Let's Encrypt.
pub fn app_with_github(db: State, github: Api) -> Router {
    router(AppState {
        github,
        ..AppState::new(db)
    })
}

fn router(state: AppState) -> Router {
    let public = Router::new()
        .route("/api/health", get(crate::routes::health::get))
        .route("/api/version", get(crate::routes::version::get))
        .merge(crate::routes::auth::router())
        .merge(crate::routes::github::public_router());

    let protected = Router::new()
        .route("/api/me", get(crate::routes::me::get))
        .merge(crate::routes::users::router())
        .merge(crate::routes::sessions::router())
        .merge(crate::routes::tokens::router())
        .merge(crate::routes::github::router())
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
