use hmac::KeyInit;
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

pub fn generate_identity() -> (StaticSecret, PublicKey) {
    let secret = StaticSecret::random();
    let public = PublicKey::from(&secret);
    (secret, public)
}

pub fn derive_shared_secret(our_secret: &StaticSecret, their_public: &PublicKey) -> [u8; 32] {
    let shared = our_secret.diffie_hellman(their_public);
    *shared.as_bytes()
}

pub fn derive_pin(shared_secret: &[u8; 32]) -> String {
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<Sha256>;

    let mut mac = HmacSha256::new_from_slice(b"manispocket-pin-v1").unwrap();
    mac.update(shared_secret);
    let result = mac.finalize().into_bytes();

    let pin_value = u32::from_be_bytes([result[0], result[1], result[2], result[3]]);
    format!("{:06}", pin_value % 1_000_000)
}

pub fn hash_content(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    hex::encode(hasher.finalize())
}

mod tests {
    #![allow(unused_imports)]
    use super::*;

    #[test]
    fn test_pin_derivation_is_deterministic() {
        let (secret_a, public_a) = generate_identity();
        let (secret_b, public_b) = generate_identity();

        let shared_a = derive_shared_secret(&secret_a, &public_b);
        let shared_b = derive_shared_secret(&secret_b, &public_a);

        assert_eq!(shared_a, shared_b);
        assert_eq!(derive_pin(&shared_a), derive_pin(&shared_b));
    }

    #[test]
    fn test_pin_is_six_digits() {
        let (secret_a, public_a) = generate_identity();
        let (secret_b, public_b) = generate_identity();
        let shared = derive_shared_secret(&secret_a, &public_b);
        let pin = derive_pin(&shared);

        assert_eq!(pin.len(), 6);
        assert!(pin.chars().all(|c| c.is_ascii_digit()));
    }
}
