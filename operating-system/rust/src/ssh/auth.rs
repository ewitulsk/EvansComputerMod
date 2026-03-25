//! SSH user authentication (RFC 4252).

use super::packet;
use crate::crypto;

/// User database entry.
pub struct UserEntry {
    pub username: String,
    pub password_hash: Option<[u8; 32]>,  // SHA-256 of password
    pub authorized_keys: Vec<[u8; 32]>,   // Ed25519 public keys
}

/// Simple user database backed by filesystem.
pub struct UserDb {
    users: Vec<UserEntry>,
}

impl UserDb {
    /// Load user database. Creates default root user if none exists.
    pub fn load() -> Self {
        let mut users = Vec::new();

        // Try to load /etc/passwd
        if let Some(content) = crate::fs::read_file_absolute("etc/passwd") {
            for line in content.lines() {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 2 {
                    let username = parts[0].to_string();
                    let password_hash = if parts[1].is_empty() {
                        None
                    } else {
                        // Password hash is stored as hex
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
                username: "root".to_string(),
                password_hash: None,
                authorized_keys: Vec::new(),
            });
        }

        // Load authorized keys for each user
        for user in &mut users {
            let path = format!("home/{}/.ssh/authorized_keys", user.username);
            if let Some(content) = crate::fs::read_file_absolute(&path) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.len() == 64 {
                        // Hex-encoded 32-byte public key
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
                None => true,  // No password set = allow any (or empty)
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

    // Username
    let (username_bytes, new_offset) = packet::decode_string(payload, offset)?;
    let username = core::str::from_utf8(username_bytes).ok()?.to_string();
    offset = new_offset;

    // Service name
    let (_service_bytes, new_offset) = packet::decode_string(payload, offset)?;
    offset = new_offset;

    // Method name
    let (method_bytes, new_offset) = packet::decode_string(payload, offset)?;
    let method = core::str::from_utf8(method_bytes).ok()?.to_string();
    offset = new_offset;

    match method.as_str() {
        "password" => {
            // bool: FALSE (not a change request)
            if offset >= payload.len() { return None; }
            let _change = payload[offset];
            offset += 1;

            let (password_bytes, _) = packet::decode_string(payload, offset)?;
            let password = core::str::from_utf8(password_bytes).ok()?.to_string();

            Some(AuthRequest::Password { username, password })
        }
        "publickey" => {
            // bool: has_signature
            if offset >= payload.len() { return None; }
            let has_signature = payload[offset] != 0;
            offset += 1;

            // Algorithm name
            let (_algo_bytes, new_offset) = packet::decode_string(payload, offset)?;
            offset = new_offset;

            // Public key blob
            let (key_blob, new_offset) = packet::decode_string(payload, offset)?;
            offset = new_offset;

            // Extract the actual 32-byte key from the blob
            // blob = [string "ssh-ed25519"] [string <32 bytes>]
            let (_, key_offset) = packet::decode_string(key_blob, 0)?;
            let (key_bytes, _) = packet::decode_string(key_blob, key_offset)?;

            let mut key = [0u8; 32];
            if key_bytes.len() == 32 {
                key.copy_from_slice(key_bytes);
            }

            let signature = if has_signature {
                let (sig_blob, _) = packet::decode_string(payload, offset)?;
                // sig_blob = [string "ssh-ed25519"] [string <64 bytes>]
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
    vec![packet::msg::USERAUTH_SUCCESS]
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

/// Set password for a user, updating /etc/passwd.
pub fn set_password(username: &str, password: &str) {
    let hash = crypto::sha256(password.as_bytes());
    let hex: String = hash.iter().map(|b| format!("{:02x}", b)).collect();

    // Read existing passwd file
    let mut entries = Vec::new();
    let mut found = false;
    if let Some(content) = crate::fs::read_file_absolute("etc/passwd") {
        for line in content.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if !parts.is_empty() && parts[0] == username {
                entries.push(format!("{}:{}", username, hex));
                found = true;
            } else {
                entries.push(line.to_string());
            }
        }
    }
    if !found {
        entries.push(format!("{}:{}", username, hex));
    }

    crate::fs::mkdir_absolute("etc");
    crate::fs::write_file_absolute("etc/passwd", &entries.join("\n"));
}
