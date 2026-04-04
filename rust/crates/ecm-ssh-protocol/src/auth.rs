//! SSH user authentication (RFC 4252).
//!
//! The `UserDb` is constructed from file contents passed in as strings,
//! rather than reading the filesystem directly.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use ecm_ssh_crypto as crypto;
use crate::packet;

/// User database entry.
pub struct UserEntry {
    pub username: String,
    pub password_hash: Option<[u8; 32]>,
    pub authorized_keys: Vec<[u8; 32]>,
}

/// Simple user database.
pub struct UserDb {
    users: Vec<UserEntry>,
}

impl UserDb {
    /// Load from the contents of /etc/passwd and per-user authorized_keys.
    ///
    /// `passwd_content`: contents of `/etc/passwd` (or None if missing).
    /// `authorized_keys_loader`: callback `fn(username) -> Option<String>` to
    ///   load `~/.ssh/authorized_keys` for a given user.
    pub fn load(
        passwd_content: Option<&str>,
        authorized_keys_loader: impl Fn(&str) -> Option<String>,
    ) -> Self {
        let mut users = Vec::new();

        if let Some(content) = passwd_content {
            for line in content.lines() {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 2 {
                    let username = String::from(parts[0]);
                    let password_hash = if parts[1].is_empty() {
                        None
                    } else {
                        hex_to_hash(parts[1])
                    };
                    users.push(UserEntry {
                        username,
                        password_hash,
                        authorized_keys: Vec::new(),
                    });
                }
            }
        }

        // Default: root user with no password
        if users.is_empty() {
            users.push(UserEntry {
                username: String::from("root"),
                password_hash: None,
                authorized_keys: Vec::new(),
            });
        }

        // Load authorized keys for each user
        for user in &mut users {
            if let Some(content) = authorized_keys_loader(&user.username) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.len() == 64 {
                        if let Some(key) = hex_to_hash(trimmed) {
                            user.authorized_keys.push(key);
                        }
                    }
                }
            }
        }

        Self { users }
    }

    /// Authenticate with password. Returns true if valid.
    pub fn check_password(&self, username: &str, password: &str) -> bool {
        if let Some(user) = self.users.iter().find(|u| u.username == username) {
            match &user.password_hash {
                None => true,
                Some(hash) => {
                    let input_hash = crypto::sha256(password.as_bytes());
                    &input_hash == hash
                }
            }
        } else {
            false
        }
    }

    /// Check if a public key is authorized for this user.
    pub fn check_public_key(&self, username: &str, key: &[u8; 32]) -> bool {
        if let Some(user) = self.users.iter().find(|u| u.username == username) {
            user.authorized_keys.contains(key)
        } else {
            false
        }
    }

    /// Check if user exists.
    pub fn user_exists(&self, username: &str) -> bool {
        self.users.iter().any(|u| u.username == username)
    }
}

/// Parse SSH_MSG_USERAUTH_REQUEST.
pub fn parse_userauth_request(payload: &[u8]) -> Option<AuthRequest> {
    if payload.is_empty() || payload[0] != packet::msg::USERAUTH_REQUEST {
        return None;
    }

    let mut offset = 1;

    let (username_bytes, new_offset) = packet::decode_string(payload, offset)?;
    let username = core::str::from_utf8(username_bytes).ok()?.to_string();
    offset = new_offset;

    let (_service_bytes, new_offset) = packet::decode_string(payload, offset)?;
    offset = new_offset;

    let (method_bytes, new_offset) = packet::decode_string(payload, offset)?;
    let method = core::str::from_utf8(method_bytes).ok()?;
    offset = new_offset;

    match method {
        "password" => {
            if offset >= payload.len() { return None; }
            let _change = payload[offset];
            offset += 1;

            let (password_bytes, _) = packet::decode_string(payload, offset)?;
            let password = core::str::from_utf8(password_bytes).ok()?.to_string();

            Some(AuthRequest::Password { username, password })
        }
        "publickey" => {
            if offset >= payload.len() { return None; }
            let has_signature = payload[offset] != 0;
            offset += 1;

            let (_algo_bytes, new_offset) = packet::decode_string(payload, offset)?;
            offset = new_offset;

            let (key_blob, new_offset) = packet::decode_string(payload, offset)?;
            offset = new_offset;

            let (_, key_offset) = packet::decode_string(key_blob, 0)?;
            let (key_bytes, _) = packet::decode_string(key_blob, key_offset)?;

            let mut key = [0u8; 32];
            if key_bytes.len() == 32 {
                key.copy_from_slice(key_bytes);
            }

            let signature = if has_signature {
                let (sig_blob, _) = packet::decode_string(payload, offset)?;
                let (_, sig_offset) = packet::decode_string(sig_blob, 0)?;
                let (sig_bytes, _) = packet::decode_string(sig_blob, sig_offset)?;
                if sig_bytes.len() == 64 {
                    let mut sig = [0u8; 64];
                    sig.copy_from_slice(sig_bytes);
                    Some(sig)
                } else {
                    None
                }
            } else {
                None
            };

            Some(AuthRequest::PublicKey { username, key, signature })
        }
        "none" => {
            Some(AuthRequest::None { username })
        }
        _ => None,
    }
}

pub enum AuthRequest {
    Password { username: String, password: String },
    PublicKey { username: String, key: [u8; 32], signature: Option<[u8; 64]> },
    None { username: String },
}

/// Build SSH_MSG_USERAUTH_SUCCESS.
pub fn build_userauth_success() -> Vec<u8> {
    alloc::vec![packet::msg::USERAUTH_SUCCESS]
}

/// Build SSH_MSG_USERAUTH_FAILURE.
pub fn build_userauth_failure(methods: &[&str], partial_success: bool) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.push(packet::msg::USERAUTH_FAILURE);
    payload.extend_from_slice(&packet::encode_name_list(methods));
    payload.push(if partial_success { 1 } else { 0 });
    payload
}

/// Build SSH_MSG_SERVICE_ACCEPT.
pub fn build_service_accept(service: &str) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.push(packet::msg::SERVICE_ACCEPT);
    payload.extend_from_slice(&packet::encode_string(service.as_bytes()));
    payload
}

/// Convert hex string to 32-byte hash.
fn hex_to_hash(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 { return None; }
    let mut result = [0u8; 32];
    for i in 0..32 {
        result[i] = u8::from_str_radix(&hex[i*2..i*2+2], 16).ok()?;
    }
    Some(result)
}

/// Hash a password with SHA-256 and return the hex string.
pub fn hash_password(password: &str) -> String {
    let hash = crypto::sha256(password.as_bytes());
    let mut hex = String::with_capacity(64);
    for b in &hash {
        hex.push_str(&format!("{:02x}", b));
    }
    hex
}
