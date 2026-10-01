use anyhow::Context;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::SecretKey;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::pkcs8::EncodePrivateKey;
use serde::Serialize;

const LIFETIME_SECS: i64 = 12 * 60 * 60;

#[derive(Serialize)]
struct Claims<'a> {
    aud: String,
    exp: i64,
    sub: &'a str,
}

pub fn secret_from_b64(b64: &str) -> anyhow::Result<SecretKey> {
    let bytes = URL_SAFE_NO_PAD
        .decode(b64)
        .context("the stored VAPID key is not base64url")?;
    SecretKey::from_slice(&bytes).context("the stored VAPID key is not a P-256 scalar")
}

pub fn public_key_b64(secret: &SecretKey) -> String {
    URL_SAFE_NO_PAD.encode(secret.public_key().to_encoded_point(false).as_bytes())
}

/// The `Authorization` value a push service wants for `endpoint`: RFC 8292 `vapid t=…, k=…`.
pub fn header(endpoint: &str, secret: &SecretKey, sub: &str, now: i64) -> anyhow::Result<String> {
    let url = reqwest::Url::parse(endpoint).context("the push endpoint is not a URL")?;
    let host = url.host_str().context("the push endpoint has no host")?;
    let aud = match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    };
    let der = secret
        .to_pkcs8_der()
        .map_err(|e| anyhow::anyhow!("encoding the VAPID key: {e}"))?;
    let jwt = jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256),
        &Claims {
            aud,
            exp: now + LIFETIME_SECS,
            sub,
        },
        &jsonwebtoken::EncodingKey::from_ec_der(der.as_bytes()),
    )
    .context("signing the VAPID token")?;
    Ok(format!("vapid t={jwt}, k={}", public_key_b64(secret)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_carries_a_token_for_the_endpoint_origin_and_the_public_key() {
        let secret = crate::push::random_secret();
        let header = header(
            "https://push.example.com:8443/send/abc",
            &secret,
            "mailto:ops@example.com",
            1_000,
        )
        .unwrap();

        let (t, k) = header
            .strip_prefix("vapid t=")
            .and_then(|rest| rest.split_once(", k="))
            .unwrap();
        assert_eq!(k, public_key_b64(&secret));
        let point = URL_SAFE_NO_PAD.decode(k).unwrap();
        assert_eq!((point.len(), point[0]), (65, 0x04));

        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES256);
        validation.validate_exp = false;
        validation.set_audience(&["https://push.example.com:8443"]);
        let public = secret.public_key().to_encoded_point(false);
        let claims = jsonwebtoken::decode::<serde_json::Value>(
            t,
            &jsonwebtoken::DecodingKey::from_ec_der(public.as_bytes()),
            &validation,
        )
        .expect("the token verifies against the public key")
        .claims;
        assert_eq!(claims["sub"], "mailto:ops@example.com");
        assert_eq!(claims["exp"], 1_000 + LIFETIME_SECS);
    }
}
