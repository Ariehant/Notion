//! Authenticated encryption — audit §2.3 and §2.4.
//!
//! XChaCha20-Poly1305 is an AEAD: it provides confidentiality **and**
//! integrity/authenticity in one primitive. The blueprint's extra "HMAC key"
//! (§2.3) is therefore redundant and removed; when we need to authenticate
//! cleartext routing/envelope metadata we bind it here as **associated data**.
//!
//! XChaCha20's 192-bit (24-byte) nonce is large enough that **random** nonces
//! are safe (§2.4) — collision probability is negligible. Every sealed message
//! generates a fresh random nonce and stores it in the envelope.

use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use super::CryptoError;

/// XChaCha20 nonce length (§2.4).
pub const NONCE_LEN: usize = 24;
/// Poly1305 tag length.
pub const TAG_LEN: usize = 16;

/// A sealed message: `nonce || ciphertext(+tag)`. Self-describing on the wire
/// and in the append-only update log, so each stored update keeps its own nonce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedBox {
    pub nonce: [u8; NONCE_LEN],
    /// Ciphertext with the 16-byte Poly1305 tag appended.
    pub ciphertext: Vec<u8>,
}

impl SealedBox {
    /// Serialize as `nonce (24) || ciphertext`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(NONCE_LEN + self.ciphertext.len());
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&self.ciphertext);
        out
    }

    /// Parse `nonce (24) || ciphertext`. Requires at least nonce + tag bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        if bytes.len() < NONCE_LEN + TAG_LEN {
            return Err(CryptoError::MalformedEnvelope);
        }
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&bytes[..NONCE_LEN]);
        Ok(SealedBox {
            nonce,
            ciphertext: bytes[NONCE_LEN..].to_vec(),
        })
    }
}

/// Seal `plaintext` under `key` with a fresh random nonce (§2.4).
pub fn seal(key: &[u8; 32], plaintext: &[u8]) -> Result<SealedBox, CryptoError> {
    seal_with_aad(key, plaintext, b"")
}

/// Seal with associated data. `aad` is authenticated but not encrypted — use it
/// for envelope headers / routing metadata (§2.3), not for secrets.
pub fn seal_with_aad(
    key: &[u8; 32],
    plaintext: &[u8],
    aad: &[u8],
) -> Result<SealedBox, CryptoError> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| CryptoError::Decryption)?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    nonce_bytes.copy_from_slice(nonce.as_slice());
    Ok(SealedBox {
        nonce: nonce_bytes,
        ciphertext,
    })
}

/// Open a sealed box (no associated data).
pub fn open(key: &[u8; 32], sealed: &SealedBox) -> Result<Vec<u8>, CryptoError> {
    open_with_aad(key, sealed, b"")
}

/// Open a sealed box, verifying `aad` matches what was sealed. A mismatch
/// (tampered header, wrong key, wrong nonce) fails authentication.
pub fn open_with_aad(
    key: &[u8; 32],
    sealed: &SealedBox,
    aad: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = XNonce::from_slice(&sealed.nonce);
    cipher
        .decrypt(
            nonce,
            Payload {
                msg: &sealed.ciphertext,
                aad,
            },
        )
        .map_err(|_| CryptoError::Decryption)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let key = [7u8; 32];
        let sealed = seal(&key, b"hello world").unwrap();
        assert_eq!(open(&key, &sealed).unwrap(), b"hello world");
    }

    #[test]
    fn nonces_are_unique_per_seal() {
        let key = [7u8; 32];
        let a = seal(&key, b"same plaintext").unwrap();
        let b = seal(&key, b"same plaintext").unwrap();
        // Fresh random nonce each time (§2.4) => different nonce and ciphertext.
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn wrong_key_fails() {
        let sealed = seal(&[1u8; 32], b"secret").unwrap();
        assert!(open(&[2u8; 32], &sealed).is_err());
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let key = [7u8; 32];
        let mut sealed = seal(&key, b"secret").unwrap();
        sealed.ciphertext[0] ^= 0xff;
        assert!(open(&key, &sealed).is_err());
    }

    #[test]
    fn aad_is_authenticated() {
        let key = [7u8; 32];
        let sealed = seal_with_aad(&key, b"body", b"header-v1").unwrap();
        // Correct AAD opens.
        assert_eq!(open_with_aad(&key, &sealed, b"header-v1").unwrap(), b"body");
        // Tampered/absent AAD fails (§2.3 — AEAD covers routing metadata).
        assert!(open_with_aad(&key, &sealed, b"header-v2").is_err());
        assert!(open(&key, &sealed).is_err());
    }

    #[test]
    fn envelope_serialization_round_trips() {
        let key = [7u8; 32];
        let sealed = seal(&key, b"payload").unwrap();
        let bytes = sealed.to_bytes();
        let parsed = SealedBox::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, sealed);
        assert_eq!(open(&key, &parsed).unwrap(), b"payload");
    }

    #[test]
    fn truncated_envelope_rejected() {
        assert!(SealedBox::from_bytes(&[0u8; 10]).is_err());
    }
}
