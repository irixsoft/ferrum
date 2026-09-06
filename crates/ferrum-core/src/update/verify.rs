use super::UpdateError;
use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub const PUBLIC_KEY_PEM: &str = include_str!("../../../../packaging/ferrum-pub.pem");

const SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

/// Reads the SPKI PEM `install.sh` inlines; anything but a plain Ed25519 key is refused.
pub fn public_key(pem: &str) -> anyhow::Result<VerifyingKey> {
    let body: String = pem
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("-----"))
        .collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(body)
        .map_err(|_| anyhow::anyhow!("the public key is not base64"))?;
    let key = der
        .strip_prefix(&SPKI_PREFIX)
        .filter(|key| key.len() == ed25519_dalek::PUBLIC_KEY_LENGTH)
        .ok_or_else(|| anyhow::anyhow!("the public key is not an Ed25519 SubjectPublicKeyInfo"))?;
    Ok(VerifyingKey::from_bytes(
        key.try_into().expect("the length was checked"),
    )?)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, pair) in text.as_bytes().chunks(2).enumerate() {
        out[i] = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

/// The line for `asset` in `sha256sum` output, as a 32-byte digest.
pub fn expected_digest(sums: &[u8], asset: &str) -> Option<[u8; 32]> {
    String::from_utf8_lossy(sums).lines().find_map(|line| {
        let mut words = line.split_whitespace();
        let digest = words.next()?;
        let name = words.next()?.trim_start_matches('*');
        (name == asset && words.next().is_none())
            .then(|| unhex(digest))
            .flatten()
    })
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// The signature is over the whole `SHA256SUMS` file, then the binary must match its line.
pub fn verify(
    key: &VerifyingKey,
    sums: &[u8],
    sig: &[u8],
    binary: &[u8],
    asset: &str,
) -> Result<(), UpdateError> {
    let signature = Signature::from_slice(sig).map_err(|_| UpdateError::BadSignature)?;
    key.verify_strict(sums, &signature)
        .map_err(|_| UpdateError::BadSignature)?;
    let expected = expected_digest(sums, asset).ok_or(UpdateError::BadChecksum)?;
    let actual: [u8; 32] = Sha256::digest(binary).into();
    if bool::from(expected.ct_eq(&actual)) {
        Ok(())
    } else {
        Err(UpdateError::BadChecksum)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    pub fn signing_key() -> SigningKey {
        let mut seed = [0u8; 32];
        rand::fill(&mut seed);
        SigningKey::from_bytes(&seed)
    }

    pub fn sums_for(entries: &[(&str, &[u8])]) -> Vec<u8> {
        entries
            .iter()
            .map(|(name, bytes)| format!("{}  {name}\n", sha256_hex(bytes)))
            .collect::<String>()
            .into_bytes()
    }

    #[test]
    fn the_real_public_key_parses_and_garbage_does_not() {
        public_key(PUBLIC_KEY_PEM).unwrap();
        assert!(public_key("-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----").is_err());
        assert!(public_key("not a pem").is_err());
        let rsa_ish = format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----",
            base64::engine::general_purpose::STANDARD.encode([0x30u8; 44])
        );
        assert!(public_key(&rsa_ish).is_err());
    }

    #[test]
    fn a_signed_sums_file_admits_only_the_named_bytes() {
        let key = signing_key();
        let binary = b"ELF binary bytes";
        let other = b"the other architecture";
        let sums = sums_for(&[
            ("ferrum-x86_64-unknown-linux-musl", binary),
            ("ferrum-aarch64-unknown-linux-musl", other),
        ]);
        let sig = key.sign(&sums).to_bytes();
        let public = key.verifying_key();
        let asset = "ferrum-x86_64-unknown-linux-musl";

        assert_eq!(verify(&public, &sums, &sig, binary, asset), Ok(()));
        assert_eq!(
            verify(
                &public,
                &sums,
                &sig,
                other,
                "ferrum-aarch64-unknown-linux-musl"
            ),
            Ok(())
        );

        let mut flipped = sig;
        flipped[10] ^= 1;
        assert_eq!(
            verify(&public, &sums, &flipped, binary, asset),
            Err(UpdateError::BadSignature)
        );
        assert_eq!(
            verify(&public, &sums, &sig[..63], binary, asset),
            Err(UpdateError::BadSignature)
        );
        let mut edited = sums.clone();
        edited[0] = b'f';
        assert_eq!(
            verify(&public, &edited, &sig, binary, asset),
            Err(UpdateError::BadSignature),
            "a changed sums file no longer matches its signature"
        );
        assert_eq!(
            verify(&public, &sums, &sig, b"tampered", asset),
            Err(UpdateError::BadChecksum)
        );
        assert_eq!(
            verify(
                &public,
                &sums,
                &sig,
                binary,
                "ferrum-riscv64gc-unknown-linux-musl"
            ),
            Err(UpdateError::BadChecksum)
        );
        assert_eq!(
            verify(&signing_key().verifying_key(), &sums, &sig, binary, asset),
            Err(UpdateError::BadSignature),
            "another key's signature is not ours"
        );
    }

    #[test]
    fn sha256sum_lines_are_read_in_both_modes_and_not_by_prefix() {
        let sums = b"ab  ferrum-x\n0000000000000000000000000000000000000000000000000000000000000001 *ferrum-x86_64-unknown-linux-musl\n";
        assert!(expected_digest(sums, "ferrum-x").is_none());
        assert!(expected_digest(sums, "ferrum-x86_64-unknown-linux-musl").is_some());
        assert!(expected_digest(sums, "ferrum-x86_64").is_none());
    }
}
