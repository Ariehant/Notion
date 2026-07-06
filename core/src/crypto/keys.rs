//! Data key + asymmetric keys — audit §2.1.
//!
//! The heart of the multi-device key-distribution gap the audit calls the
//! "single biggest gap":
//!
//! * [`DataKey`] (DEK) — a random 256-bit key, **independent of the password**.
//!   Everything at rest/in transit is ultimately protected by the DEK, so a
//!   password change only re-wraps it (no full re-encryption).
//! * [`IdentityKeypair`] — a per-user **Ed25519** keypair. Backs the relay's
//!   signature-based challenge (§2.3) and signs device-enrollment grants.
//! * [`DeviceKeypair`] — a per-device **X25519** keypair. A new device
//!   publishes its public key; an existing device *wraps the DEK to it*
//!   ([`WrappedDek`], an X25519 sealed box). This is how device #2 obtains the
//!   key without the DEK ever touching the relay in the clear.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use x25519_dalek::{PublicKey as XPublicKey, SharedSecret, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::aead::{open, seal, SealedBox};
use super::kdf::hkdf_subkey;
use super::{CryptoError, KEY_LEN};

/// The 256-bit data-encryption key. Zeroized on drop; stays in Rust (§2.6).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct DataKey([u8; KEY_LEN]);

impl DataKey {
    /// Generate a fresh random DEK from the OS CSPRNG.
    pub fn generate() -> Self {
        use rand::RngCore;
        let mut k = [0u8; KEY_LEN];
        OsRng.fill_bytes(&mut k);
        DataKey(k)
    }

    /// Construct from raw bytes (e.g. after unwrapping).
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        DataKey(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// Wrap the DEK with a symmetric key derived from the password path
    /// (the `dek_wrap` HKDF subkey). Password change ⇒ re-wrap only (§2.1).
    pub fn wrap_with_key(&self, wrap_key: &[u8; KEY_LEN]) -> Result<SealedBox, CryptoError> {
        seal(wrap_key, &self.0)
    }

    /// Recover the DEK from a password-path wrap.
    pub fn unwrap_with_key(
        wrap_key: &[u8; KEY_LEN],
        sealed: &SealedBox,
    ) -> Result<Self, CryptoError> {
        let bytes = open(wrap_key, sealed)?;
        let arr: [u8; KEY_LEN] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| CryptoError::InvalidKey)?;
        Ok(DataKey(arr))
    }

    /// Derive the at-rest content keys **from the DEK** (not the password).
    ///
    /// This is what makes the DEK the true root of content protection (§2.1):
    /// SQLCipher's page-encryption key and the sync-update AEAD key both hang off
    /// the DEK via HKDF with distinct `info` labels, so changing the password only
    /// re-wraps the DEK — it never re-keys the database or re-seals the log.
    pub fn content_keys(&self) -> ContentKeys {
        ContentKeys {
            sqlcipher: hkdf_subkey(&self.0, b"notion.v1.dek.sqlcipher"),
            sync_aead: hkdf_subkey(&self.0, b"notion.v1.dek.sync-aead"),
        }
    }
}

/// The two at-rest content keys derived from the DEK. Zeroized on drop.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct ContentKeys {
    /// Raw 256-bit key handed to SQLCipher (`PRAGMA key = "x'..'"`).
    pub sqlcipher: [u8; KEY_LEN],
    /// Key used to seal/open sync-update log entries.
    pub sync_aead: [u8; KEY_LEN],
}

impl ContentKeys {
    /// The SQLCipher key as the 64-char lowercase hex SQLCipher expects.
    pub fn sqlcipher_hex(&self) -> String {
        hex::encode(self.sqlcipher)
    }
}

/// A device's X25519 public key (shareable), 32 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DevicePublicKey(pub [u8; 32]);

impl DevicePublicKey {
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

/// A device keypair used to receive a wrapped DEK during pairing (§2.1).
#[derive(ZeroizeOnDrop)]
pub struct DeviceKeypair {
    secret: StaticSecret,
    #[zeroize(skip)]
    public: XPublicKey,
}

impl DeviceKeypair {
    /// Generate a fresh device keypair.
    pub fn generate() -> Self {
        let secret = StaticSecret::random_from_rng(OsRng);
        let public = XPublicKey::from(&secret);
        DeviceKeypair { secret, public }
    }

    pub fn public(&self) -> DevicePublicKey {
        DevicePublicKey(self.public.to_bytes())
    }

    /// Unwrap a [`WrappedDek`] that was sealed to *this* device's public key.
    pub fn unwrap_dek(&self, wrapped: &WrappedDek) -> Result<DataKey, CryptoError> {
        let epk = XPublicKey::from(wrapped.ephemeral_pub);
        let shared = self.secret.diffie_hellman(&epk);
        // Defensive: reject a degenerate (all-zero) shared secret that a
        // low-order `ephemeral_pub` in a malicious wrap could force.
        let shared = contributory(shared)?;
        let aead_key = dek_seal_key(&shared, &wrapped.ephemeral_pub, &self.public.to_bytes());
        let bytes = open(&aead_key, &wrapped.sealed)?;
        let arr: [u8; KEY_LEN] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| CryptoError::InvalidKey)?;
        Ok(DataKey(arr))
    }
}

/// A DEK wrapped (sealed) to a specific device's public key (§2.1).
///
/// Constructed via [`WrappedDek::seal_to`]. The `ephemeral_pub` is a one-time
/// X25519 public key; combined with the recipient's static key it yields the
/// AEAD key. Both public keys are bound into the HKDF `info` so a wrap cannot be
/// replayed against a different recipient (unknown-key-share resistance).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedDek {
    pub ephemeral_pub: [u8; 32],
    pub recipient_pub: [u8; 32],
    pub sealed: SealedBox,
}

impl WrappedDek {
    /// Wrap `dek` so that only the holder of `recipient`'s secret can open it.
    pub fn seal_to(dek: &DataKey, recipient: &DevicePublicKey) -> Result<Self, CryptoError> {
        let ephemeral = StaticSecret::random_from_rng(OsRng);
        let ephemeral_pub = XPublicKey::from(&ephemeral).to_bytes();
        let recipient_pk = XPublicKey::from(recipient.0);
        // Reject a degenerate agreement (e.g. a low-order recipient key).
        let shared = contributory(ephemeral.diffie_hellman(&recipient_pk))?;
        let aead_key = dek_seal_key(&shared, &ephemeral_pub, &recipient.0);
        let sealed = seal(&aead_key, dek.as_bytes())?;
        Ok(WrappedDek {
            ephemeral_pub,
            recipient_pub: recipient.0,
            sealed,
        })
    }
}

/// Reject non-contributory X25519 agreements (all-zero shared secret produced
/// by low-order points), which carry no entropy from our secret.
fn contributory(shared: SharedSecret) -> Result<[u8; 32], CryptoError> {
    if shared.was_contributory() {
        Ok(shared.to_bytes())
    } else {
        Err(CryptoError::WeakKeyAgreement)
    }
}

/// Derive the sealed-box AEAD key, binding both public keys into HKDF `info`.
fn dek_seal_key(shared: &[u8; 32], ephemeral_pub: &[u8; 32], recipient_pub: &[u8; 32]) -> [u8; 32] {
    let mut info = Vec::with_capacity(19 + 64);
    info.extend_from_slice(b"notion.v1.dek-seal");
    info.push(0);
    info.extend_from_slice(ephemeral_pub);
    info.extend_from_slice(recipient_pub);
    hkdf_subkey(shared, &info)
}

/// A user's Ed25519 public key (device-enrollment / relay-auth identity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentityPublicKey(pub [u8; 32]);

impl IdentityPublicKey {
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    fn verifying(&self) -> Result<VerifyingKey, CryptoError> {
        VerifyingKey::from_bytes(&self.0).map_err(|_| CryptoError::InvalidKey)
    }

    /// Verify a relay challenge signature (§2.3 — signature-based challenge).
    pub fn verify_challenge(&self, challenge: &[u8], sig: &[u8; 64]) -> Result<(), CryptoError> {
        let vk = self.verifying()?;
        let signature = Signature::from_bytes(sig);
        vk.verify(challenge, &signature)
            .map_err(|_| CryptoError::BadSignature)
    }
}

/// The per-user Ed25519 identity keypair (§2.1, §2.3).
#[derive(ZeroizeOnDrop)]
pub struct IdentityKeypair {
    #[zeroize(skip)]
    signing: SigningKey,
}

impl IdentityKeypair {
    pub fn generate() -> Self {
        IdentityKeypair {
            signing: SigningKey::generate(&mut OsRng),
        }
    }

    /// Restore from a stored 32-byte seed (should itself be sealed at rest).
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        IdentityKeypair {
            signing: SigningKey::from_bytes(seed),
        }
    }

    pub fn seed(&self) -> [u8; 32] {
        self.signing.to_bytes()
    }

    pub fn public(&self) -> IdentityPublicKey {
        IdentityPublicKey(self.signing.verifying_key().to_bytes())
    }

    /// Sign a relay-issued challenge to prove identity (§2.3).
    pub fn sign_challenge(&self, challenge: &[u8]) -> [u8; 64] {
        self.signing.sign(challenge).to_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_path_wrap_round_trip() {
        let dek = DataKey::generate();
        let wrap_key = [9u8; KEY_LEN];
        let sealed = dek.wrap_with_key(&wrap_key).unwrap();
        let back = DataKey::unwrap_with_key(&wrap_key, &sealed).unwrap();
        assert_eq!(dek.as_bytes(), back.as_bytes());
    }

    #[test]
    fn device_wrap_round_trip() {
        // Existing device holds the DEK; new device publishes a pubkey.
        let dek = DataKey::generate();
        let new_device = DeviceKeypair::generate();

        let wrapped = WrappedDek::seal_to(&dek, &new_device.public()).unwrap();
        let recovered = new_device.unwrap_dek(&wrapped).unwrap();

        assert_eq!(dek.as_bytes(), recovered.as_bytes());
    }

    #[test]
    fn dek_cannot_be_unwrapped_by_wrong_device() {
        let dek = DataKey::generate();
        let intended = DeviceKeypair::generate();
        let attacker = DeviceKeypair::generate();

        let wrapped = WrappedDek::seal_to(&dek, &intended.public()).unwrap();
        assert!(attacker.unwrap_dek(&wrapped).is_err());
    }

    #[test]
    fn low_order_ephemeral_is_rejected() {
        // A malicious wrap using an all-zero (low-order) ephemeral key must not
        // decrypt to anything; the non-contributory agreement is rejected.
        let device = DeviceKeypair::generate();
        let wrapped = WrappedDek {
            ephemeral_pub: [0u8; 32],
            recipient_pub: device.public().0,
            sealed: seal(&[0u8; 32], b"x").unwrap(),
        };
        assert!(matches!(
            device.unwrap_dek(&wrapped),
            Err(CryptoError::WeakKeyAgreement)
        ));
    }

    #[test]
    fn generated_deks_differ() {
        assert_ne!(
            DataKey::generate().as_bytes(),
            DataKey::generate().as_bytes()
        );
    }

    #[test]
    fn content_keys_are_dek_rooted_distinct_and_stable() {
        let dek = DataKey::generate();
        let a = dek.content_keys();
        let b = dek.content_keys();
        // Stable for a given DEK (so the DB reopens with the same key).
        assert_eq!(a.sqlcipher, b.sqlcipher);
        assert_eq!(a.sync_aead, b.sync_aead);
        // The two purposes are cryptographically separated (§2.2).
        assert_ne!(a.sqlcipher, a.sync_aead);
        // A different DEK yields different content keys (rooted in the DEK).
        let other = DataKey::generate().content_keys();
        assert_ne!(a.sqlcipher, other.sqlcipher);
        // SQLCipher hex is exactly 64 lowercase hex chars.
        let hex = a.sqlcipher_hex();
        assert_eq!(hex.len(), 64);
        assert!(hex.bytes().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn identity_challenge_round_trip() {
        let id = IdentityKeypair::generate();
        let pubkey = id.public();
        let challenge = b"relay-nonce-12345";
        let sig = id.sign_challenge(challenge);
        assert!(pubkey.verify_challenge(challenge, &sig).is_ok());
        // Wrong challenge fails.
        assert!(pubkey.verify_challenge(b"different-nonce", &sig).is_err());
    }

    #[test]
    fn identity_seed_round_trips() {
        let id = IdentityKeypair::generate();
        let restored = IdentityKeypair::from_seed(&id.seed());
        assert_eq!(id.public(), restored.public());
    }

    #[test]
    fn wrong_identity_key_rejected() {
        let id = IdentityKeypair::generate();
        let other = IdentityKeypair::generate();
        let challenge = b"nonce";
        let sig = id.sign_challenge(challenge);
        assert!(other.public().verify_challenge(challenge, &sig).is_err());
    }
}
