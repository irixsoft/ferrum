use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

pub const BYTES: usize = 32;

pub fn generate() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; BYTES]>())
}

pub fn hash(presented: &str) -> String {
    Sha256::digest(presented.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_secrets_do_not_repeat() {
        let a = generate();
        let b = generate();
        assert_ne!(a, b);
        assert!(a.len() >= 43, "{a} is shorter than 256 bits of entropy");
    }

    #[test]
    fn hashing_is_stable_and_hides_the_input() {
        let token = generate();
        assert_eq!(hash(&token), hash(&token));
        assert_ne!(hash(&token), token);
        assert_eq!(hash(&token).len(), 64);
    }

    #[test]
    fn different_inputs_hash_differently() {
        assert_ne!(hash("a"), hash("b"));
    }
}
