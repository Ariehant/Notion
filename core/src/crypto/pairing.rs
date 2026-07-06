//! Device pairing — audit §2.1 (Phase-0 spike #5), hardened after review.
//!
//! When a new device joins, two problems must be solved together:
//!   1. **MITM detection** — both devices display a Short Authentication String
//!      ([`sas_code`]) that the user compares out-of-band (like Signal's safety
//!      number). A relay that swapped keys produces a different SAS.
//!   2. **Key transfer** — the existing device issues a signed [`PairingGrant`]
//!      carrying the DEK *wrapped to the new device's public key*. The DEK never
//!      reaches the relay in the clear.
//!
//! ## Why commit-then-reveal (fixes the "grindable SAS" finding)
//!
//! A naive SAS = `H(deviceKeyA ‖ deviceKeyB)` is **grindable**: an active relay
//! can pick its substituted keys *after* seeing the victims' keys and brute-force
//! a ~36-bit collision (seconds of work) so both displayed strings match. It also
//! failed to bind the Ed25519 **identity** that [`PairingGrant::accept`] anchors
//! trust on, so the attacker could swap that for free.
//!
//! This module fixes both:
//!   * Each side contributes a fresh random **nonce** and first exchanges a
//!     hash **commitment** ([`PairingContribution::commitment`]) before revealing
//!     its contribution. Neither side can choose keys/nonce after seeing the
//!     peer's, so the SAS cannot be ground.
//!   * The SAS transcript binds **both device keys, both identity keys, and both
//!     nonces** — confirming the SAS authenticates the authorizer identity too.

use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::keys::{
    DataKey, DeviceKeypair, DevicePublicKey, IdentityKeypair, IdentityPublicKey, WrappedDek,
};
use super::CryptoError;

/// A 64-word list (6 bits/word) for rendering the SAS.
pub const SAS_WORDS: [&str; 64] = [
    "acid", "apple", "arrow", "atlas", "amber", "anvil", "angel", "auburn", "basin", "beacon",
    "birch", "blaze", "brook", "cabin", "cedar", "chart", "clover", "comet", "coral", "crane",
    "delta", "dune", "ember", "fable", "falcon", "fern", "flint", "gale", "glade", "grove",
    "harbor", "hazel", "ivory", "jade", "kite", "lagoon", "lark", "lotus", "maple", "meadow",
    "moss", "nectar", "oak", "onyx", "opal", "otter", "pearl", "pine", "quartz", "quill", "raven",
    "reef", "ridge", "sable", "spruce", "stone", "tide", "topaz", "umber", "vine", "willow",
    "wren", "yarrow", "zephyr",
];

/// Number of SAS words users compare (≈36 bits with a 64-word list). With the
/// commit-reveal exchange the attacker gets a single un-ground online guess, so
/// this bounds MITM success at ≈2⁻³⁶ per pairing.
pub const SAS_WORD_COUNT: usize = 6;

/// Each device's public pairing contribution: its keys plus a fresh nonce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingContribution {
    pub device_pub: [u8; 32],
    pub identity_pub: [u8; 32],
    pub nonce: [u8; 32],
}

impl PairingContribution {
    /// Build a contribution with a freshly sampled random nonce.
    pub fn generate(device: &DevicePublicKey, identity: &IdentityPublicKey) -> Self {
        let mut nonce = [0u8; 32];
        OsRng.fill_bytes(&mut nonce);
        PairingContribution {
            device_pub: device.0,
            identity_pub: identity.0,
            nonce,
        }
    }

    /// The hash commitment sent BEFORE revealing this contribution.
    pub fn commitment(&self) -> PairingCommitment {
        let mut h = Sha256::new();
        h.update(b"notion.v1.pairing-commit");
        h.update([0u8]);
        h.update(self.device_pub);
        h.update(self.identity_pub);
        h.update(self.nonce);
        PairingCommitment(h.finalize().into())
    }
}

/// A hiding commitment to a [`PairingContribution`], exchanged first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingCommitment(pub [u8; 32]);

/// Verify a revealed contribution matches the commitment received earlier.
/// A mismatch means the peer changed its keys/nonce after committing (MITM /
/// tamper) — abort the pairing.
pub fn verify_commitment(
    revealed: &PairingContribution,
    commitment: &PairingCommitment,
) -> Result<(), CryptoError> {
    let expected = revealed.commitment();
    if expected.0.ct_eq(&commitment.0).unwrap_u8() == 1 {
        Ok(())
    } else {
        Err(CryptoError::PairingCommitmentMismatch)
    }
}

/// Derive the human-comparable Short Authentication String from the full
/// pairing transcript. Order-independent: both devices compute the same value.
/// Binds both device keys, both identity keys, and both nonces.
pub fn sas_code(local: &PairingContribution, remote: &PairingContribution) -> Vec<&'static str> {
    // Canonical ordering by device key so both ends hash the same transcript.
    let (a, b) = if local.device_pub <= remote.device_pub {
        (local, remote)
    } else {
        (remote, local)
    };
    let mut h = Sha256::new();
    h.update(b"notion.v1.sas");
    for c in [a, b] {
        h.update(c.device_pub);
        h.update(c.identity_pub);
        h.update(c.nonce);
    }
    let digest = h.finalize();
    (0..SAS_WORD_COUNT)
        .map(|i| SAS_WORDS[(digest[i] & 0b0011_1111) as usize])
        .collect()
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

    fn contribution() -> (DeviceKeypair, IdentityKeypair, PairingContribution) {
        let dev = DeviceKeypair::generate();
        let id = IdentityKeypair::generate();
        let c = PairingContribution::generate(&dev.public(), &id.public());
        (dev, id, c)
    }

    #[test]
    fn sas_is_order_independent_and_stable() {
        let (_, _, a) = contribution();
        let (_, _, b) = contribution();
        assert_eq!(sas_code(&a, &b), sas_code(&b, &a));
        assert_eq!(sas_code(&a, &b).len(), SAS_WORD_COUNT);
    }

    #[test]
    fn sas_binds_identity_and_nonce() {
        let (_, _, a) = contribution();
        let (_, _, b) = contribution();
        let base = sas_code(&a, &b);

        // Swapping the peer identity key changes the SAS (identity is bound).
        let mut b2 = b.clone();
        b2.identity_pub[0] ^= 0xff;
        assert_ne!(sas_code(&a, &b2), base);

        // Changing the peer nonce changes the SAS (freshness is bound).
        let mut b3 = b.clone();
        b3.nonce[0] ^= 0xff;
        assert_ne!(sas_code(&a, &b3), base);
    }

    #[test]
    fn commitment_detects_post_commit_key_change() {
        let (_, _, honest) = contribution();
        let commitment = honest.commitment();
        assert!(verify_commitment(&honest, &commitment).is_ok());

        // Attacker committed to `honest` but reveals different keys → detected.
        let mut swapped = honest.clone();
        swapped.device_pub[0] ^= 0xff;
        assert!(matches!(
            verify_commitment(&swapped, &commitment),
            Err(CryptoError::PairingCommitmentMismatch)
        ));
    }

    #[test]
    fn full_commit_reveal_and_grant_flow() {
        // Existing device owns the DEK + the user's identity key.
        let existing_dev = DeviceKeypair::generate();
        let identity = IdentityKeypair::generate();
        let dek = DataKey::generate();
        let existing_c = PairingContribution::generate(&existing_dev.public(), &identity.public());

        // New device generates its keypair + a throwaway pairing identity.
        let new_dev = DeviceKeypair::generate();
        let new_id = IdentityKeypair::generate();
        let new_c = PairingContribution::generate(&new_dev.public(), &new_id.public());

        // 1. Exchange commitments, then reveal + verify against them.
        let existing_commit = existing_c.commitment();
        let new_commit = new_c.commitment();
        verify_commitment(&new_c, &new_commit).unwrap();
        verify_commitment(&existing_c, &existing_commit).unwrap();

        // 2. Both compute the SAS over the transcript and it matches.
        assert_eq!(sas_code(&existing_c, &new_c), sas_code(&new_c, &existing_c));

        // 3. Existing device issues the signed grant; new device accepts.
        let grant = PairingGrant::issue(&identity, &dek, &new_dev.public()).unwrap();
        let recovered = grant.accept(&new_dev, &identity.public()).unwrap();
        assert_eq!(recovered.as_bytes(), dek.as_bytes());
    }

    #[test]
    fn grant_from_untrusted_authorizer_rejected() {
        let real = IdentityKeypair::generate();
        let attacker = IdentityKeypair::generate();
        let dek = DataKey::generate();
        let new_device = DeviceKeypair::generate();

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
