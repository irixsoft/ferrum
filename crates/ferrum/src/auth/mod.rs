pub mod webauthn;

use crate::routes::error::ApiError;
use crate::server::AppState;
use axum::extract::{FromRequestParts, Request, State as Extract};
use axum::http::HeaderMap;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::Response;
use ferrum_core::sessions;
use ferrum_core::tokens::{self, ApiToken};
use ferrum_core::users::User;

const SIGN_IN: &str = "Sign in to use this.";

#[derive(Debug, Clone)]
pub enum Caller {
    User(User),
    Machine(ApiToken),
}

impl Caller {
    pub fn is_read_only(&self) -> bool {
        matches!(self, Caller::Machine(token) if token.read_only)
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Caller::User(_) => "user",
            Caller::Machine(_) => "machine",
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Caller::User(user) => &user.name,
            Caller::Machine(token) => &token.name,
        }
    }

    pub fn user(&self) -> Option<&User> {
        match self {
            Caller::User(user) => Some(user),
            Caller::Machine(_) => None,
        }
    }
}

const READ_ONLY: &str = "That API token is read-only.";

pub async fn require_caller(
    Extract(app): Extract<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let caller = resolve(&app, request.headers()).await?;
    if caller.is_read_only() && !request.method().is_safe() {
        return Err(ApiError::new(axum::http::StatusCode::FORBIDDEN, READ_ONLY));
    }
    request.extensions_mut().insert(caller);
    Ok(next.run(request).await)
}

pub async fn resolve(app: &AppState, headers: &HeaderMap) -> Result<Caller, ApiError> {
    if let Some(presented) = bearer(headers) {
        return tokens::verify(&app.db, &presented)
            .await?
            .map(Caller::Machine)
            .ok_or_else(|| ApiError::unauthorized("That API token is not valid."));
    }

    let token = cookie(headers, sessions::COOKIE).ok_or_else(|| ApiError::unauthorized(SIGN_IN))?;
    sessions::resolve(&app.db, &token)
        .await?
        .map(Caller::User)
        .ok_or_else(|| ApiError::unauthorized(SIGN_IN))
}

impl FromRequestParts<AppState> for Caller {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        _app: &AppState,
    ) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<Caller>().cloned().ok_or_else(|| {
            tracing::error!(
                path = %parts.uri.path(),
                "a route asked for the caller without sitting behind the auth layer"
            );
            ApiError::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "This route is misconfigured.",
            )
        })
    }
}

pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| k.trim() == name)
        .map(|(_, v)| v.trim().to_string())
}

pub fn bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// nginx sets `X-Real-IP`, and the daemon is loopback-only, so nothing else can set it.
pub fn device(headers: &HeaderMap) -> sessions::Device<'_> {
    sessions::Device {
        user_agent: header(headers, axum::http::header::USER_AGENT.as_str()),
        ip: header(headers, "x-real-ip"),
    }
}

fn header<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

pub fn session_cookie(token: &str) -> String {
    format!(
        "{}={token}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={}",
        sessions::COOKIE,
        sessions::TTL_DAYS * 24 * 60 * 60
    )
}

pub fn cleared_session_cookie() -> String {
    format!(
        "{}=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0",
        sessions::COOKIE
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(axum::http::HeaderName, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (name, value) in pairs {
            h.append(name.clone(), HeaderValue::from_str(value).unwrap());
        }
        h
    }

    #[test]
    fn a_cookie_is_found_among_others() {
        let h = headers(&[(
            axum::http::header::COOKIE,
            "theme=dark; ferrum_session=abc123; other=1",
        )]);
        assert_eq!(cookie(&h, "ferrum_session").as_deref(), Some("abc123"));
        assert_eq!(cookie(&h, "absent"), None);
    }

    #[test]
    fn a_cookie_is_found_across_repeated_headers() {
        let h = headers(&[
            (axum::http::header::COOKIE, "theme=dark"),
            (axum::http::header::COOKIE, "ferrum_session=abc123"),
        ]);
        assert_eq!(cookie(&h, "ferrum_session").as_deref(), Some("abc123"));
    }

    #[test]
    fn a_prefix_does_not_match_a_different_cookie() {
        let h = headers(&[(axum::http::header::COOKIE, "ferrum_session_old=abc123")]);
        assert_eq!(cookie(&h, "ferrum_session"), None);
    }

    #[test]
    fn a_bearer_token_is_read_and_a_bare_header_is_not() {
        let h = headers(&[(axum::http::header::AUTHORIZATION, "Bearer ferr_abc")]);
        assert_eq!(bearer(&h).as_deref(), Some("ferr_abc"));

        let basic = headers(&[(axum::http::header::AUTHORIZATION, "Basic ferr_abc")]);
        assert_eq!(bearer(&basic), None);

        let empty = headers(&[(axum::http::header::AUTHORIZATION, "Bearer ")]);
        assert_eq!(bearer(&empty), None);
    }

    #[test]
    fn the_session_cookie_is_locked_down() {
        let value = session_cookie("abc123");
        for flag in ["HttpOnly", "Secure", "SameSite=Strict", "Path=/"] {
            assert!(value.contains(flag), "{value} is missing {flag}");
        }
        assert!(value.starts_with("ferrum_session=abc123;"));
    }

    #[test]
    fn clearing_the_cookie_expires_it_immediately() {
        assert!(cleared_session_cookie().contains("Max-Age=0"));
    }
}
