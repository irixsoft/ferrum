use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes128Gcm, Nonce};
use anyhow::Context;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hkdf::Hkdf;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::{PublicKey, SecretKey, ecdh};
use sha2::Sha256;

const RECORD_SIZE: u32 = 4096;

/// RFC 8291 `aes128gcm`: one record, for the browser that owns `p256dh` and `auth`.
pub fn encrypt(payload: &[u8], p256dh_b64: &str, auth_b64: &str) -> anyhow::Result<Vec<u8>> {
    encrypt_with(
        payload,
        p256dh_b64,
        auth_b64,
        &super::random_secret(),
        &rand::random(),
    )
}

fn encrypt_with(
    payload: &[u8],
    p256dh_b64: &str,
    auth_b64: &str,
    as_secret: &SecretKey,
    salt: &[u8; 16],
) -> anyhow::Result<Vec<u8>> {
    let ua_public_bytes = URL_SAFE_NO_PAD
        .decode(p256dh_b64.trim_end_matches('='))
        .context("the subscription's p256dh is not base64url")?;
    let ua_public = PublicKey::from_sec1_bytes(&ua_public_bytes)
        .context("the subscription's p256dh is not a P-256 point")?;
    let auth = URL_SAFE_NO_PAD
        .decode(auth_b64.trim_end_matches('='))
        .context("the subscription's auth is not base64url")?;
    let shared = ecdh::diffie_hellman(as_secret.to_nonzero_scalar(), ua_public.as_affine());
    let as_public = as_secret.public_key().to_encoded_point(false);

    let mut info = b"WebPush: info\0".to_vec();
    info.extend_from_slice(&ua_public_bytes);
    info.extend_from_slice(as_public.as_bytes());
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(&auth), shared.raw_secret_bytes())
        .expand(&info, &mut ikm)
        .map_err(|e| anyhow::anyhow!("deriving the push key: {e}"))?;

    let hk = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let mut cek = [0u8; 16];
    let mut nonce = [0u8; 12];
    hk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .and_then(|()| hk.expand(b"Content-Encoding: nonce\0", &mut nonce))
        .map_err(|e| anyhow::anyhow!("deriving the push key: {e}"))?;

    let mut record = payload.to_vec();
    record.push(0x02);
    let sealed = Aes128Gcm::new_from_slice(&cek)
        .expect("a 16-byte key")
        .encrypt(Nonce::from_slice(&nonce), record.as_slice())
        .map_err(|e| anyhow::anyhow!("sealing the push payload: {e}"))?;

    let key_id = as_public.as_bytes();
    let mut body = Vec::with_capacity(21 + key_id.len() + sealed.len());
    body.extend_from_slice(salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(key_id.len() as u8);
    body.extend_from_slice(key_id);
    body.extend_from_slice(&sealed);
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decrypt(body: &[u8], ua_secret: &SecretKey, auth: &[u8]) -> Vec<u8> {
        let salt = &body[..16];
        let id_len = body[20] as usize;
        let as_public = PublicKey::from_sec1_bytes(&body[21..21 + id_len]).unwrap();
        let shared = ecdh::diffie_hellman(ua_secret.to_nonzero_scalar(), as_public.as_affine());
        let mut info = b"WebPush: info\0".to_vec();
        info.extend_from_slice(ua_secret.public_key().to_encoded_point(false).as_bytes());
        info.extend_from_slice(as_public.to_encoded_point(false).as_bytes());
        let mut ikm = [0u8; 32];
        Hkdf::<Sha256>::new(Some(auth), shared.raw_secret_bytes())
            .expand(&info, &mut ikm)
            .unwrap();
        let hk = Hkdf::<Sha256>::new(Some(salt), &ikm);
        let mut cek = [0u8; 16];
        let mut nonce = [0u8; 12];
        hk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
            .unwrap();
        hk.expand(b"Content-Encoding: nonce\0", &mut nonce).unwrap();
        let mut record = Aes128Gcm::new_from_slice(&cek)
            .unwrap()
            .decrypt(Nonce::from_slice(&nonce), &body[21 + id_len..])
            .expect("the browser can open the record");
        assert_eq!(record.pop(), Some(0x02), "the last-record delimiter");
        record
    }

    #[test]
    fn the_browser_holding_the_subscription_keys_opens_the_payload() {
        let ua_secret = crate::push::random_secret();
        let p256dh =
            URL_SAFE_NO_PAD.encode(ua_secret.public_key().to_encoded_point(false).as_bytes());
        let auth: [u8; 16] = rand::random();
        let payload =
            serde_json::json!({"title": "Deploy live", "body": "ledger", "link": "/apps/ledger"});

        let body = encrypt(
            payload.to_string().as_bytes(),
            &p256dh,
            &URL_SAFE_NO_PAD.encode(auth),
        )
        .unwrap();

        assert_eq!(&body[16..20], &RECORD_SIZE.to_be_bytes());
        assert_eq!(body[20], 65);
        assert_eq!(body[21], 0x04);
        let opened: serde_json::Value =
            serde_json::from_slice(&decrypt(&body, &ua_secret, &auth)).unwrap();
        assert_eq!(opened, payload);
    }
}
