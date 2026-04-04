//! SSH transport layer state machine.

use alloc::string::String;
use alloc::vec::Vec;

use crate::packet;
use crate::kex;

/// State of the SSH transport.
pub enum TransportState {
    /// Waiting for version exchange.
    VersionExchange,
    /// Version exchanged, waiting for KEXINIT.
    WaitingForKexInit,
    /// KEXINIT sent, waiting for peer's KEXINIT.
    KexInitSent,
    /// Key exchange in progress.
    KexInProgress {
        our_kexinit: Vec<u8>,
        peer_kexinit: Vec<u8>,
    },
    /// Keys established, encrypted transport ready.
    Established,
}

/// An SSH transport session.
pub struct SshTransport {
    pub state: TransportState,
    pub is_server: bool,
    pub our_version: String,
    pub peer_version: String,
    pub session_id: Option<[u8; 32]>,
    pub encrypt_key: Option<[u8; 32]>,
    pub decrypt_key: Option<[u8; 32]>,
    pub encrypt_iv: Option<[u8; 32]>,
    pub decrypt_iv: Option<[u8; 32]>,
    pub send_seq: u32,
    pub recv_seq: u32,
    pub recv_buffer: Vec<u8>,
}

impl SshTransport {
    pub fn new(is_server: bool) -> Self {
        Self {
            state: TransportState::VersionExchange,
            is_server,
            our_version: kex::VERSION_STRING.into(),
            peer_version: String::new(),
            session_id: None,
            encrypt_key: None,
            decrypt_key: None,
            encrypt_iv: None,
            decrypt_iv: None,
            send_seq: 0,
            recv_seq: 0,
            recv_buffer: Vec::new(),
        }
    }

    /// Get the version string to send (with \r\n).
    pub fn version_line(&self) -> Vec<u8> {
        let mut line = self.our_version.as_bytes().to_vec();
        line.extend_from_slice(b"\r\n");
        line
    }

    /// Process received version line. Returns true if valid.
    pub fn set_peer_version(&mut self, line: &str) -> bool {
        let trimmed = line.trim_end_matches(|c| c == '\r' || c == '\n');
        if trimmed.starts_with("SSH-2.0-") {
            self.peer_version = trimmed.into();
            self.state = TransportState::WaitingForKexInit;
            true
        } else {
            false
        }
    }

    /// Build and return our KEXINIT packet bytes.
    pub fn build_kexinit_packet(&mut self) -> Vec<u8> {
        let payload = kex::build_kexinit();
        let pkt = packet::encode_packet(&payload);

        if let TransportState::WaitingForKexInit = &self.state {
            self.state = TransportState::KexInitSent;
        }

        self.send_seq += 1;
        pkt
    }

    /// Encode a payload into a packet (handles encryption after NEWKEYS).
    pub fn encode_packet(&mut self, payload: &[u8]) -> Vec<u8> {
        let pkt = if self.encrypt_key.is_some() {
            // TODO: encrypt with ChaCha20-Poly1305
            packet::encode_packet(payload)
        } else {
            packet::encode_packet(payload)
        };
        self.send_seq += 1;
        pkt
    }

    /// Feed received data into the transport buffer.
    /// Returns decoded payloads.
    pub fn feed(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        self.recv_buffer.extend_from_slice(data);
        let mut payloads = Vec::new();

        loop {
            if self.decrypt_key.is_some() {
                // TODO: decrypt with ChaCha20-Poly1305
                match packet::decode_packet(&self.recv_buffer) {
                    Some((payload, consumed)) => {
                        self.recv_buffer.drain(..consumed);
                        self.recv_seq += 1;
                        payloads.push(payload);
                    }
                    None => break,
                }
            } else {
                match packet::decode_packet(&self.recv_buffer) {
                    Some((payload, consumed)) => {
                        self.recv_buffer.drain(..consumed);
                        self.recv_seq += 1;
                        payloads.push(payload);
                    }
                    None => break,
                }
            }
        }

        payloads
    }

    /// Set encryption keys after successful key exchange.
    pub fn set_keys(
        &mut self,
        shared_secret: &[u8; 32],
        exchange_hash: &[u8; 32],
    ) {
        let session_id = self.session_id.unwrap_or(*exchange_hash);

        let iv_c2s = kex::derive_key(shared_secret, exchange_hash, b'A', &session_id);
        let iv_s2c = kex::derive_key(shared_secret, exchange_hash, b'B', &session_id);
        let key_c2s = kex::derive_key(shared_secret, exchange_hash, b'C', &session_id);
        let key_s2c = kex::derive_key(shared_secret, exchange_hash, b'D', &session_id);

        if self.is_server {
            self.encrypt_key = Some(key_s2c);
            self.decrypt_key = Some(key_c2s);
            self.encrypt_iv = Some(iv_s2c);
            self.decrypt_iv = Some(iv_c2s);
        } else {
            self.encrypt_key = Some(key_c2s);
            self.decrypt_key = Some(key_s2c);
            self.encrypt_iv = Some(iv_c2s);
            self.decrypt_iv = Some(iv_s2c);
        }

        if self.session_id.is_none() {
            self.session_id = Some(*exchange_hash);
        }

        self.state = TransportState::Established;
    }
}
