//! SSH host key management using filesystem host functions.

use ecm_host_abi::fs;
use ecm_ssh_crypto as crypto;
use ecm_ssh_crypto::{Ed25519VerifyingKey, Ed25519SigningKey};

const HOST_KEY_PRIV_PATH: &str = "etc/ssh/ssh_host_ed25519_key";
const HOST_KEY_PUB_PATH: &str = "etc/ssh/ssh_host_ed25519_key.pub";

/// Load or generate the SSH host key.
pub fn load_or_generate_host_key() -> (Ed25519VerifyingKey, Ed25519SigningKey) {
    // Try to load existing private key
    if let Some(key_bytes) = fs::read_file_bytes(HOST_KEY_PRIV_PATH) {
        if key_bytes.len() == 32 {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&key_bytes);
            let signing_key = crypto::ed25519_private_key_from_bytes(&arr);
            let verifying_key = signing_key.verifying_key();
            return (verifying_key, signing_key);
        }
    }

    generate_and_save_host_key()
}

fn generate_and_save_host_key() -> (Ed25519VerifyingKey, Ed25519SigningKey) {
    let (verifying_key, signing_key) = crypto::ed25519_generate_keypair();

    fs::mkdir("etc/ssh");

    fs::write_file_bytes(HOST_KEY_PRIV_PATH, &crypto::ed25519_private_key_bytes(&signing_key));

    let pub_bytes = crypto::ed25519_public_key_bytes(&verifying_key);
    let mut pub_data = pub_bytes.to_vec();
    pub_data.extend_from_slice(b" ssh-ed25519 host-key\n");
    fs::write_file_bytes(HOST_KEY_PUB_PATH, &pub_data);

    (verifying_key, signing_key)
}
