//! Chain-of-custody with hybrid Ed25519 signatures.
//!
//! Every evidence-handling event is a checkpoint in an append-only
//! hash chain. Each checkpoint is signed with Ed25519. The hybrid
//! with ML-DSA (FIPS 204) is added when a stable pure-Rust
//! implementation is available; for now we keep Ed25519 and reserve
//! the signature-slot in the JSON for forward compatibility.

use chrono::Utc;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureBundle {
    pub ed25519: String,
    pub ed_pk: String,
    /// Reserved for ML-DSA (FIPS 204) once a stable pure-Rust impl ships.
    pub ml_dsa: Option<String>,
    pub ml_pk: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub prev: String,
    pub ts: String,
    pub event: serde_json::Value,
    pub hash: String,
    pub sig: SignatureBundle,
}

pub struct CustodySigner {
    sk: SigningKey,
}

impl CustodySigner {
    pub fn generate() -> Self {
        Self { sk: SigningKey::generate(&mut OsRng) }
    }

    pub fn sign(&self, msg: &[u8]) -> SignatureBundle {
        let sig = self.sk.sign(msg);
        SignatureBundle {
            ed25519: hex::encode(sig.to_bytes()),
            ed_pk: hex::encode(self.sk.verifying_key().to_bytes()),
            ml_dsa: None,
            ml_pk: None,
        }
    }

    pub fn verifying_key_hex(&self) -> String {
        hex::encode(self.sk.verifying_key().to_bytes())
    }
}

pub fn verify(msg: &[u8], bundle: &SignatureBundle) -> bool {
    let pk = match hex::decode(&bundle.ed_pk) { Ok(v) => v, Err(_) => return false };
    let sig_bytes = match hex::decode(&bundle.ed25519) { Ok(v) => v, Err(_) => return false };

    let pk_arr: [u8; 32] = match pk.try_into() { Ok(a) => a, Err(_) => return false };
    let sig_arr: [u8; 64] = match sig_bytes.try_into() { Ok(a) => a, Err(_) => return false };

    let vk = match VerifyingKey::from_bytes(&pk_arr) { Ok(v) => v, Err(_) => return false };
    let sig = Signature::from_bytes(&sig_arr);
    vk.verify(msg, &sig).is_ok()
}

/// Append a new checkpoint to the chain.
pub fn checkpoint(
    prev_hash: &str,
    event: serde_json::Value,
    signer: &CustodySigner,
) -> Checkpoint {
    let body = serde_json::json!({
        "prev": prev_hash,
        "ts": Utc::now().to_rfc3339(),
                                 "event": event,
    });
    let canonical = serde_json::to_vec(&body).unwrap();

    let hash = hex::encode(Sha256::digest(&canonical));
    let sig = signer.sign(&canonical);

    Checkpoint {
        prev: prev_hash.to_string(),
        ts: body["ts"].as_str().unwrap().to_string(),
        event: body["event"].clone(),
        hash,
        sig,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_sign_verify() {
        let signer = CustodySigner::generate();
        let msg = b"hello, forensic world";
        let bundle = signer.sign(msg);
        assert!(verify(msg, &bundle));
        assert!(!verify(b"tampered", &bundle));
    }

    #[test]
    fn chain_links() {
        let signer = CustodySigner::generate();
        let c1 = checkpoint(GENESIS_HASH, serde_json::json!({"action": "acquire"}), &signer);
        let c2 = checkpoint(&c1.hash, serde_json::json!({"action": "analyze"}), &signer);
        assert_eq!(c2.prev, c1.hash);
        assert_ne!(c1.hash, c2.hash);
    }
}
