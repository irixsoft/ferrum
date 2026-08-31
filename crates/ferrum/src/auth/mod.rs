pub mod webauthn;

use axum::http::HeaderMap;
use ferrum_core::sessions;

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

pub fn user_agent(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
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
