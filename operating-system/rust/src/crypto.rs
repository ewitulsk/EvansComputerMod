//! Cryptographic primitives for SSH.
//!
//! Provides Ed25519 signing, X25519 key exchange, ChaCha20-Poly1305 AEAD,
//! SHA-256, and HMAC-SHA-256. All implementations are pure Rust and compile
//! to wasm32-unknown-unknown.

use sha2::{Sha256, Digest};
use hmac::{Hmac, Mac};
use ed25519_dalek::{SigningKey, VerifyingKey, Signer, Verifier, Signature};
use crate::fs;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};

/// Generate an Ed25519 signing keypair.
/// Uses the custom getrandom implementation for WASM entropy.
pub fn ed25519_generate_keypair() -> (VerifyingKey, SigningKey) {
    let mut rng_bytes = [0u8; 32];
    getrandom::getrandom(&mut rng_bytes).expect("getrandom failed");
    let signing_key = SigningKey::from_bytes(&rng_bytes);
    let verifying_key = signing_key.verifying_key();
    (verifying_key, signing_key)
}

/// Sign a message with Ed25519.
pub fn ed25519_sign(signing_key: &SigningKey, message: &[u8]) -> [u8; 64] {
    let sig = signing_key.sign(message);
    sig.to_bytes()
}

/// Verify an Ed25519 signature.
pub fn ed25519_verify(verifying_key: &VerifyingKey, message: &[u8], signature: &[u8; 64]) -> bool {
    let sig = Signature::from_bytes(signature);
    verifying_key.verify(message, &sig).is_ok()
}

/// Serialize an Ed25519 verifying (public) key to 32 bytes.
pub fn ed25519_public_key_bytes(key: &VerifyingKey) -> [u8; 32] {
    key.to_bytes()
}

/// Deserialize an Ed25519 verifying (public) key from 32 bytes.
pub fn ed25519_public_key_from_bytes(bytes: &[u8; 32]) -> Result<VerifyingKey, &'static str> {
    VerifyingKey::from_bytes(bytes).map_err(|_| "invalid public key")
}

/// Serialize an Ed25519 signing (private) key to 32 bytes.
pub fn ed25519_private_key_bytes(key: &SigningKey) -> [u8; 32] {
    key.to_bytes()
}

/// Deserialize an Ed25519 signing (private) key from 32 bytes.
pub fn ed25519_private_key_from_bytes(bytes: &[u8; 32]) -> SigningKey {
    SigningKey::from_bytes(bytes)
}

/// Generate an X25519 static secret and its public key.
pub fn x25519_generate_keypair() -> (X25519PublicKey, StaticSecret) {
    let mut rng_bytes = [0u8; 32];
    getrandom::getrandom(&mut rng_bytes).expect("getrandom failed");
    let secret = StaticSecret::from(rng_bytes);
    let public = X25519PublicKey::from(&secret);
    (public, secret)
}

/// Perform X25519 Diffie-Hellman key agreement.
pub fn x25519_diffie_hellman(our_secret: &StaticSecret, their_public: &X25519PublicKey) -> [u8; 32] {
    our_secret.diffie_hellman(their_public).to_bytes()
}

/// Compute SHA-256 hash.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// Compute SHA-256 hash of multiple data chunks.
pub fn sha256_multi(chunks: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for chunk in chunks {
        hasher.update(chunk);
    }
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// Compute HMAC-SHA-256.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC key size");
    mac.update(data);
    let result = mac.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result.into_bytes());
    out
}

/// Encrypt with ChaCha20-Poly1305.
/// Returns ciphertext + 16-byte tag appended.
pub fn chacha20_poly1305_encrypt(key: &[u8; 32], nonce: &[u8; 12], plaintext: &[u8]) -> Result<Vec<u8>, &'static str> {
    let cipher = ChaCha20Poly1305::new(key.into());
    let nonce = Nonce::from_slice(nonce);
    cipher.encrypt(nonce, plaintext).map_err(|_| "encryption failed")
}

/// Decrypt with ChaCha20-Poly1305.
/// Input is ciphertext + 16-byte tag.
pub fn chacha20_poly1305_decrypt(key: &[u8; 32], nonce: &[u8; 12], ciphertext: &[u8]) -> Result<Vec<u8>, &'static str> {
    let cipher = ChaCha20Poly1305::new(key.into());
    let nonce = Nonce::from_slice(nonce);
    cipher.decrypt(nonce, ciphertext).map_err(|_| "decryption failed")
}

/// Path to the SSH host Ed25519 private key.
const HOST_KEY_PRIV_PATH: &str = "etc/ssh/ssh_host_ed25519_key";
/// Path to the SSH host Ed25519 public key.
const HOST_KEY_PUB_PATH: &str = "etc/ssh/ssh_host_ed25519_key.pub";

/// Load or generate the SSH host key.
///
/// On first boot the key is generated and persisted to the filesystem at
/// `/etc/ssh/ssh_host_ed25519_key` (private, 32 bytes) and
/// `/etc/ssh/ssh_host_ed25519_key.pub` (public, 32 bytes + comment).
/// On subsequent boots the existing key is loaded.
pub fn load_or_generate_host_key() -> (VerifyingKey, SigningKey) {
    // Try to load an existing private key
    if let Some(key_bytes) = fs::read_file_bytes_absolute(HOST_KEY_PRIV_PATH) {
        if key_bytes.len() == 32 {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&key_bytes);
            let signing_key = ed25519_private_key_from_bytes(&arr);
            let verifying_key = signing_key.verifying_key();
            return (verifying_key, signing_key);
        }
    }

    // Generate a fresh keypair
    generate_and_save_host_key()
}

/// Generate a new SSH host key, overwriting any existing one.
pub fn generate_and_save_host_key() -> (VerifyingKey, SigningKey) {
    let (verifying_key, signing_key) = ed25519_generate_keypair();

    // Ensure the directory exists
    fs::mkdir_absolute("etc/ssh");

    // Save the 32-byte private key seed
    fs::write_file_bytes_absolute(HOST_KEY_PRIV_PATH, &ed25519_private_key_bytes(&signing_key));

    // Save the public key (32 raw bytes + human-readable comment)
    let pub_bytes = ed25519_public_key_bytes(&verifying_key);
    let mut pub_data = pub_bytes.to_vec();
    pub_data.extend_from_slice(b" ssh-ed25519 host-key\n");
    fs::write_file_bytes_absolute(HOST_KEY_PUB_PATH, &pub_data);

    (verifying_key, signing_key)
}
