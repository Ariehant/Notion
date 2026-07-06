//! Device pairing — audit §2.1 (Phase-0 spike #5).
//!
//! When a new device joins, two problems must be solved together:
//!   1. **MITM detection** — both devices display a Short Authentication String
//!      ([`sas_code`]) derived from the pairing transcript. The user confirms
//!      the strings match out-of-band (like Signal's safety number). A relay
//!      that swapped keys would produce a different SAS.
//!   2. **Key transfer** — the existing device issues a signed [`PairingGrant`]
//!      that carries the DEK *wrapped to the new device's public key*. The DEK
//!      never reaches the relay in the clear.

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::keys::{
    DataKey, DeviceKeypair, DevicePublicKey, IdentityKeypair, IdentityPublicKey, WrappedDek,
};
use super::CryptoError;

/// A 64-word list (6 bits/word) for rendering the SAS. Two devices comparing
/// six of these words verify ~36 bits of the transcript hash.
pub const SAS_WORDS: [&str; 64] = [
    "acid", "apple", "arrow", "atlas", "amber", "anvil", "angel", "auburn", "basin", "beacon",
    "birch", "blaze", "brook", "cabin", "cedar", "chart", "clover", "comet", "coral", "crane",
    "delta", "dune", "ember", "fable", "falcon", "fern", "flint", "gale", "glade", "grove",
    "harbor", "hazel", "ivory", "jade", "kite", "lagoon", "lark", "lotus", "maple", "meadow",
    "moss", "nectar", "oak", "onyx", "opal", "otter", "pearl", "pine", "quartz", "quill", "raven",
    "reef", "ridge", "sable", "spruce", "stone", "tide", "topaz", "umber", "vine", "willow",
    "wren", "yarrow", "zephyr",
];

/// Number of SAS words users compare (≈36 bits with a 64-word list).
const SAS_LEN: usize = 6;

/// Derive the human-comparable Short Authentication String for a pairing
/// between two public keys. Order-independent: both devices compute the same
/// value regardless of which key they call `a` vs `b`.
pub fn sas_code(a: &[u8; 32], b: &[u8; 32]) -> Vec<&'static str> {
    // Sort so the transcript is canonical on both ends.
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let mut hasher = Sha256::new();
    hasher.update(b"notion.v1.sas");
    hasher.update(lo);
    hasher.update(hi);
    let digest = hasher.finalize();

    // Take 6 bits per word from the digest.
    let mut words = Vec::with_capacity(SAS_LEN);
    for i in 0..SAS_LEN {
        let idx = (digest[i] & 0b0011_1111) as usize;
        words.push(SAS_WORDS[idx]);
    }
    words
}

/// A signed authorization that transfers the DEK to a new device (§2.1).
#[derive(Debug, Clone)]
pub struct PairingGrant {
    /// The new device's public key the DEK was wrapped to.
    pub recipient_device_pub: [u8; 32],
    /// The DEK, sealed to `recipient_device_pub`.
    pub wrapped_dek: WrappedDek,
    /// Identity that authorized the enrollment (an already-trusted device).
    pub signed_by: IdentityPublicKey,
    /// Ed25519 signature over the grant transcript.
    pub signature: [u8; 64],
}

impl PairingGrant {
    /// Domain-separated transcript that the signature covers.
    fn transcript(recipient: &[u8; 32], wrapped: &WrappedDek) -> Vec<u8> {
        let mut t = Vec::new();
        t.extend_from_slice(b"notion.v1.pairing-grant");
        t.push(0);
        t.extend_from_slice(recipient);
        t.extend_from_slice(&wrapped.ephemeral_pub);
        t.extend_from_slice(&wrapped.sealed.to_bytes());
        t
    }

    /// Existing device: wrap the DEK to `recipient` and sign the grant.
    pub fn issue(
        authorizer: &IdentityKeypair,
        dek: &DataKey,
        recipient: &DevicePublicKey,
    ) -> Result<Self, CryptoError> {
        let wrapped = WrappedDek::seal_to(dek, recipient)?;
        let transcript = Self::transcript(&recipient.0, &wrapped);
        let signature = authorizer.sign_challenge(&transcript);
        Ok(PairingGrant {
            recipient_device_pub: recipient.0,
            wrapped_dek: wrapped,
            signed_by: authorizer.public(),
            signature,
        })
    }

    /// New device: verify the grant was signed by a *trusted* authorizer
    /// (identity confirmed out-of-band via the SAS), then unwrap the DEK.
    pub fn accept(
        &self,
        device: &DeviceKeypair,
        trusted_authorizer: &IdentityPublicKey,
    ) -> Result<DataKey, CryptoError> {
        // The signer must be the identity the user verified via SAS.
        if self.signed_by.0.ct_eq(&trusted_authorizer.0).unwrap_u8() != 1 {
            return Err(CryptoError::BadSignature);
        }
        // The grant must target this device.
        if self
            .recipient_device_pub
            .ct_eq(&device.public().0)
            .unwrap_u8()
            != 1
        {
            return Err(CryptoError::InvalidKey);
        }
        let transcript = Self::transcript(&self.recipient_device_pub, &self.wrapped_dek);
        self.signed_by
            .verify_challenge(&transcript, &self.signature)?;
        device.unwrap_dek(&self.wrapped_dek)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sas_is_order_independent_and_stable() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        assert_eq!(sas_code(&a, &b), sas_code(&b, &a));
        assert_eq!(sas_code(&a, &b).len(), SAS_LEN);
    }

    #[test]
    fn sas_differs_for_different_keys() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        let c = [3u8; 32];
        // Overwhelmingly likely to differ; a swapped key (MITM) shows a mismatch.
        assert_ne!(sas_code(&a, &b), sas_code(&a, &c));
    }

    #[test]
    fn full_pairing_flow() {
        // Existing device owns the DEK + the user's identity key.
        let identity = IdentityKeypair::generate();
        let dek = DataKey::generate();

        // New device generates its keypair and shares the public key.
        let new_device = DeviceKeypair::generate();

        // Existing device issues the signed grant.
        let grant = PairingGrant::issue(&identity, &dek, &new_device.public()).unwrap();

        // New device accepts, trusting the identity confirmed via SAS.
        let recovered = grant.accept(&new_device, &identity.public()).unwrap();
        assert_eq!(recovered.as_bytes(), dek.as_bytes());
    }

    #[test]
    fn grant_from_untrusted_authorizer_rejected() {
        let real = IdentityKeypair::generate();
        let attacker = IdentityKeypair::generate();
        let dek = DataKey::generate();
        let new_device = DeviceKeypair::generate();

        // Attacker issues a grant with their own identity.
        let grant = PairingGrant::issue(&attacker, &dek, &new_device.public()).unwrap();

        // Device trusts only the real identity → rejected.
        assert!(grant.accept(&new_device, &real.public()).is_err());
    }

    #[test]
    fn tampered_grant_signature_rejected() {
        let identity = IdentityKeypair::generate();
        let dek = DataKey::generate();
        let new_device = DeviceKeypair::generate();
        let mut grant = PairingGrant::issue(&identity, &dek, &new_device.public()).unwrap();
        grant.signature[0] ^= 0xff;
        assert!(grant.accept(&new_device, &identity.public()).is_err());
    }
}
