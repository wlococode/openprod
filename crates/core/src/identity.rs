use ed25519_dalek::{Signer, Verifier};

use crate::error::CoreError;
use crate::ids::{ActorId, Signature};

pub struct ActorIdentity {
    signing_key: ed25519_dalek::SigningKey,
}

impl ActorIdentity {
    pub fn generate() -> Self {
        let mut rng = rand::thread_rng();
        Self {
            signing_key: ed25519_dalek::SigningKey::generate(&mut rng),
        }
    }

    pub fn from_secret_bytes(bytes: &[u8; 32]) -> Self {
        Self {
            signing_key: ed25519_dalek::SigningKey::from_bytes(bytes),
        }
    }

    pub fn secret_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    pub fn actor_id(&self) -> ActorId {
        let verifying_key = self.signing_key.verifying_key();
        ActorId::from_bytes(verifying_key.to_bytes())
    }

    pub fn sign(&self, message: &[u8]) -> Signature {
        let sig = self.signing_key.sign(message);
        Signature::from_bytes(sig.to_bytes())
    }
}

pub fn verify_signature(
    actor_id: &ActorId,
    message: &[u8],
    signature: &Signature,
) -> Result<(), CoreError> {
    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(actor_id.as_bytes())
        .map_err(|_| CoreError::InvalidSignature)?;
    let sig = ed25519_dalek::Signature::from_bytes(signature.as_bytes());
    verifying_key
        .verify(message, &sig)
        .map_err(|_| CoreError::InvalidSignature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_roundtrip() {
        let identity = ActorIdentity::generate();
        let message = b"hello world";
        let sig = identity.sign(message);
        assert!(verify_signature(&identity.actor_id(), message, &sig).is_ok());
    }

    #[test]
    fn wrong_message_fails() {
        let identity = ActorIdentity::generate();
        let sig = identity.sign(b"message A");
        assert!(verify_signature(&identity.actor_id(), b"message B", &sig).is_err());
    }

    #[test]
    fn wrong_key_fails() {
        let identity_a = ActorIdentity::generate();
        let identity_b = ActorIdentity::generate();
        let message = b"test message";
        let sig = identity_a.sign(message);
        assert!(verify_signature(&identity_b.actor_id(), message, &sig).is_err());
    }

    #[test]
    fn secret_bytes_roundtrip() {
        let identity = ActorIdentity::generate();
        let bytes = identity.secret_bytes();
        let restored = ActorIdentity::from_secret_bytes(&bytes);
        assert_eq!(identity.actor_id(), restored.actor_id());
    }
}
