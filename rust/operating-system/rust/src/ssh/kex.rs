//! SSH key exchange using curve25519-sha256 (RFC 8731).

use crate::crypto;
use super::packet;

/// SSH version string for this implementation.
pub const VERSION_STRING: &str = "SSH-2.0-TerminalOS_1.0";

/// Supported algorithms.
pub const KEX_ALGORITHMS: &[&str] = &["curve25519-sha256"];
pub const HOST_KEY_ALGORITHMS: &[&str] = &["ssh-ed25519"];
pub const CIPHER_ALGORITHMS: &[&str] = &["chacha20-poly1305@openssh.com", "none"];
pub const MAC_ALGORITHMS: &[&str] = &["none"]; // Implicit with AEAD
pub const COMPRESSION_ALGORITHMS: &[&str] = &["none"];

/// Build an SSH_MSG_KEXINIT packet payload.
pub fn build_kexinit() -> Vec<u8> {
    let mut payload = Vec::new();

    // Message type
    payload.push(packet::msg::KEXINIT);

    // Cookie (16 random bytes)
    let mut cookie = [0u8; 16];
    let _ = getrandom::getrandom(&mut cookie);
    payload.extend_from_slice(&cookie);

    // Algorithm lists
    payload.extend_from_slice(&packet::encode_name_list(KEX_ALGORITHMS));
    payload.extend_from_slice(&packet::encode_name_list(HOST_KEY_ALGORITHMS));
    payload.extend_from_slice(&packet::encode_name_list(CIPHER_ALGORITHMS)); // client-to-server
    payload.extend_from_slice(&packet::encode_name_list(CIPHER_ALGORITHMS)); // server-to-client
    payload.extend_from_slice(&packet::encode_name_list(MAC_ALGORITHMS)); // client-to-server
    payload.extend_from_slice(&packet::encode_name_list(MAC_ALGORITHMS)); // server-to-client
    payload.extend_from_slice(&packet::encode_name_list(COMPRESSION_ALGORITHMS)); // client-to-server
    payload.extend_from_slice(&packet::encode_name_list(COMPRESSION_ALGORITHMS)); // server-to-client

    // Languages (empty)
    payload.extend_from_slice(&packet::encode_name_list(&[]));
    payload.extend_from_slice(&packet::encode_name_list(&[]));

    // First kex packet follows: false
    payload.push(0);

    // Reserved (uint32)
    payload.extend_from_slice(&[0, 0, 0, 0]);

    payload
}

/// Parsed KEXINIT data.
pub struct KexInitData {
    pub kex_algorithms: Vec<String>,
    pub host_key_algorithms: Vec<String>,
    pub raw_payload: Vec<u8>,
}

/// Parse a KEXINIT packet to extract algorithm lists.
pub fn parse_kexinit(payload: &[u8]) -> Option<KexInitData> {
    if payload.is_empty() || payload[0] != packet::msg::KEXINIT {
        return None;
    }

    // Skip type (1) + cookie (16)
    let mut offset = 17;

    let (kex_algorithms, new_offset) = packet::decode_name_list(payload, offset)?;
    offset = new_offset;

    let (host_key_algorithms, new_offset) = packet::decode_name_list(payload, offset)?;
    offset = new_offset;

    // Skip remaining algorithm lists (6 more)
    for _ in 0..6 {
        let (_, new_offset) = packet::decode_name_list(payload, offset)?;
        offset = new_offset;
    }

    Some(KexInitData {
        kex_algorithms: kex_algorithms.iter().map(|s| s.to_string()).collect(),
        host_key_algorithms: host_key_algorithms.iter().map(|s| s.to_string()).collect(),
        raw_payload: payload.to_vec(),
    })
}

/// Compute the SSH exchange hash H for curve25519-sha256.
/// H = SHA-256(V_C || V_S || I_C || I_S || K_S || Q_C || Q_S || K)
pub fn compute_exchange_hash(
    client_version: &str,
    server_version: &str,
    client_kexinit: &[u8],  // raw KEXINIT payload
    server_kexinit: &[u8],  // raw KEXINIT payload
    host_key_blob: &[u8],   // encoded host public key
    client_ephemeral_pub: &[u8; 32],  // Q_C
    server_ephemeral_pub: &[u8; 32],  // Q_S
    shared_secret: &[u8; 32],        // K
) -> [u8; 32] {
    let mut data = Vec::new();

    // V_C (client version string, as SSH string)
    data.extend_from_slice(&packet::encode_string(client_version.as_bytes()));
    // V_S (server version string, as SSH string)
    data.extend_from_slice(&packet::encode_string(server_version.as_bytes()));
    // I_C (client KEXINIT payload, as SSH string)
    data.extend_from_slice(&packet::encode_string(client_kexinit));
    // I_S (server KEXINIT payload, as SSH string)
    data.extend_from_slice(&packet::encode_string(server_kexinit));
    // K_S (host key blob, as SSH string)
    data.extend_from_slice(&packet::encode_string(host_key_blob));
    // Q_C (client ephemeral public key, as SSH string)
    data.extend_from_slice(&packet::encode_string(client_ephemeral_pub));
    // Q_S (server ephemeral public key, as SSH string)
    data.extend_from_slice(&packet::encode_string(server_ephemeral_pub));
    // K (shared secret, as mpint)
    data.extend_from_slice(&packet::encode_mpint(shared_secret));

    crypto::sha256(&data)
}

/// Derive session keys from K (shared secret) and H (exchange hash).
/// key = SHA-256(K || H || letter || session_id)
/// where letter is 'A'-'F' for different keys.
pub fn derive_key(shared_secret: &[u8; 32], hash: &[u8; 32], letter: u8, session_id: &[u8; 32]) -> [u8; 32] {
    let mut data = Vec::new();
    data.extend_from_slice(&packet::encode_mpint(shared_secret));
    data.extend_from_slice(hash);
    data.push(letter);
    data.extend_from_slice(session_id);
    crypto::sha256(&data)
}

/// Encode an Ed25519 public key in SSH format.
/// Format: [string "ssh-ed25519"] [string <32-byte key>]
pub fn encode_ed25519_public_key(key_bytes: &[u8; 32]) -> Vec<u8> {
    let mut blob = Vec::new();
    blob.extend_from_slice(&packet::encode_string(b"ssh-ed25519"));
    blob.extend_from_slice(&packet::encode_string(key_bytes));
    blob
}

/// Encode an Ed25519 signature in SSH format.
/// Format: [string "ssh-ed25519"] [string <64-byte signature>]
pub fn encode_ed25519_signature(signature: &[u8; 64]) -> Vec<u8> {
    let mut blob = Vec::new();
    blob.extend_from_slice(&packet::encode_string(b"ssh-ed25519"));
    blob.extend_from_slice(&packet::encode_string(signature));
    blob
}
