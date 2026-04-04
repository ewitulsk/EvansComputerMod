//! SSH connection protocol — channels (RFC 4254).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::packet;

/// Maximum number of concurrent channels.
pub const MAX_CHANNELS: usize = 8;

/// SSH channel state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChannelState {
    Open,
    Closed,
}

/// An SSH channel.
pub struct Channel {
    pub local_id: u32,
    pub remote_id: u32,
    pub state: ChannelState,
    pub local_window: u32,
    pub remote_window: u32,
    pub local_max_packet: u32,
    pub remote_max_packet: u32,
    pub tty_id: Option<u32>,
    pub rx_buffer: Vec<u8>,
    pub tx_buffer: Vec<u8>,
}

impl Channel {
    pub fn new(local_id: u32, remote_id: u32, remote_window: u32, remote_max_packet: u32) -> Self {
        Self {
            local_id,
            remote_id,
            state: ChannelState::Open,
            local_window: 32768,
            remote_window,
            local_max_packet: 32768,
            remote_max_packet,
            tty_id: None,
            rx_buffer: Vec::new(),
            tx_buffer: Vec::new(),
        }
    }
}

/// Channel manager.
pub struct ChannelManager {
    channels: Vec<Option<Channel>>,
    next_id: u32,
}

impl ChannelManager {
    pub fn new() -> Self {
        Self {
            channels: (0..MAX_CHANNELS).map(|_| None).collect(),
            next_id: 0,
        }
    }

    /// Allocate a new channel. Returns local channel ID or None.
    pub fn open(&mut self, remote_id: u32, remote_window: u32, remote_max_packet: u32) -> Option<u32> {
        for slot in self.channels.iter_mut() {
            if slot.is_none() {
                let local_id = self.next_id;
                self.next_id += 1;
                *slot = Some(Channel::new(local_id, remote_id, remote_window, remote_max_packet));
                return Some(local_id);
            }
        }
        None
    }

    /// Get a channel by local ID.
    pub fn get(&self, local_id: u32) -> Option<&Channel> {
        self.channels.iter()
            .flatten()
            .find(|c| c.local_id == local_id)
    }

    /// Get a mutable channel by local ID.
    pub fn get_mut(&mut self, local_id: u32) -> Option<&mut Channel> {
        self.channels.iter_mut()
            .flatten()
            .find(|c| c.local_id == local_id)
    }

    /// Close a channel.
    pub fn close(&mut self, local_id: u32) {
        for slot in &mut self.channels {
            if let Some(ch) = slot {
                if ch.local_id == local_id {
                    ch.state = ChannelState::Closed;
                    *slot = None;
                    return;
                }
            }
        }
    }
}

// ---- Packet builders ----

/// Build SSH_MSG_CHANNEL_OPEN_CONFIRMATION.
pub fn build_channel_open_confirmation(
    remote_id: u32,
    local_id: u32,
    initial_window: u32,
    max_packet: u32,
) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.push(packet::msg::CHANNEL_OPEN_CONFIRMATION);
    payload.extend_from_slice(&remote_id.to_be_bytes());
    payload.extend_from_slice(&local_id.to_be_bytes());
    payload.extend_from_slice(&initial_window.to_be_bytes());
    payload.extend_from_slice(&max_packet.to_be_bytes());
    payload
}

/// Build SSH_MSG_CHANNEL_OPEN_FAILURE.
pub fn build_channel_open_failure(remote_id: u32, reason: u32, description: &str) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.push(packet::msg::CHANNEL_OPEN_FAILURE);
    payload.extend_from_slice(&remote_id.to_be_bytes());
    payload.extend_from_slice(&reason.to_be_bytes());
    payload.extend_from_slice(&packet::encode_string(description.as_bytes()));
    payload.extend_from_slice(&packet::encode_string(b""));
    payload
}

/// Build SSH_MSG_CHANNEL_DATA.
pub fn build_channel_data(remote_id: u32, data: &[u8]) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.push(packet::msg::CHANNEL_DATA);
    payload.extend_from_slice(&remote_id.to_be_bytes());
    payload.extend_from_slice(&packet::encode_string(data));
    payload
}

/// Build SSH_MSG_CHANNEL_WINDOW_ADJUST.
pub fn build_window_adjust(remote_id: u32, bytes_to_add: u32) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.push(packet::msg::CHANNEL_WINDOW_ADJUST);
    payload.extend_from_slice(&remote_id.to_be_bytes());
    payload.extend_from_slice(&bytes_to_add.to_be_bytes());
    payload
}

/// Build SSH_MSG_CHANNEL_EOF.
pub fn build_channel_eof(remote_id: u32) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.push(packet::msg::CHANNEL_EOF);
    payload.extend_from_slice(&remote_id.to_be_bytes());
    payload
}

/// Build SSH_MSG_CHANNEL_CLOSE.
pub fn build_channel_close(remote_id: u32) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.push(packet::msg::CHANNEL_CLOSE);
    payload.extend_from_slice(&remote_id.to_be_bytes());
    payload
}

/// Build SSH_MSG_CHANNEL_SUCCESS.
pub fn build_channel_success(remote_id: u32) -> Vec<u8> {
    alloc::vec![packet::msg::CHANNEL_SUCCESS,
         (remote_id >> 24) as u8, (remote_id >> 16) as u8,
         (remote_id >> 8) as u8, remote_id as u8]
}

/// Build SSH_MSG_CHANNEL_FAILURE.
pub fn build_channel_failure(remote_id: u32) -> Vec<u8> {
    alloc::vec![packet::msg::CHANNEL_FAILURE,
         (remote_id >> 24) as u8, (remote_id >> 16) as u8,
         (remote_id >> 8) as u8, remote_id as u8]
}

/// Build SSH_MSG_CHANNEL_OPEN for session type.
pub fn build_channel_open_session(sender_channel: u32, initial_window: u32, max_packet: u32) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.push(packet::msg::CHANNEL_OPEN);
    payload.extend_from_slice(&packet::encode_string(b"session"));
    payload.extend_from_slice(&sender_channel.to_be_bytes());
    payload.extend_from_slice(&initial_window.to_be_bytes());
    payload.extend_from_slice(&max_packet.to_be_bytes());
    payload
}

/// Build SSH_MSG_CHANNEL_REQUEST for pty-req.
pub fn build_pty_request(
    recipient_channel: u32,
    term: &str,
    width_chars: u32,
    height_rows: u32,
    width_pixels: u32,
    height_pixels: u32,
) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.push(packet::msg::CHANNEL_REQUEST);
    payload.extend_from_slice(&recipient_channel.to_be_bytes());
    payload.extend_from_slice(&packet::encode_string(b"pty-req"));
    payload.push(1); // want reply
    payload.extend_from_slice(&packet::encode_string(term.as_bytes()));
    payload.extend_from_slice(&width_chars.to_be_bytes());
    payload.extend_from_slice(&height_rows.to_be_bytes());
    payload.extend_from_slice(&width_pixels.to_be_bytes());
    payload.extend_from_slice(&height_pixels.to_be_bytes());
    payload.extend_from_slice(&packet::encode_string(b"")); // encoded terminal modes
    payload
}

/// Build SSH_MSG_CHANNEL_REQUEST for shell.
pub fn build_shell_request(recipient_channel: u32) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.push(packet::msg::CHANNEL_REQUEST);
    payload.extend_from_slice(&recipient_channel.to_be_bytes());
    payload.extend_from_slice(&packet::encode_string(b"shell"));
    payload.push(1); // want reply
    payload
}

// ---- Packet parsers ----

/// Parse SSH_MSG_CHANNEL_OPEN.
pub fn parse_channel_open(payload: &[u8]) -> Option<ChannelOpenRequest> {
    if payload.is_empty() || payload[0] != packet::msg::CHANNEL_OPEN {
        return None;
    }
    let (channel_type, offset) = packet::decode_string(payload, 1)?;
    let channel_type = core::str::from_utf8(channel_type).ok()?.to_string();
    let (sender_channel, offset) = packet::decode_u32(payload, offset)?;
    let (initial_window, offset) = packet::decode_u32(payload, offset)?;
    let (max_packet, _) = packet::decode_u32(payload, offset)?;

    Some(ChannelOpenRequest {
        channel_type,
        sender_channel,
        initial_window,
        max_packet,
    })
}

pub struct ChannelOpenRequest {
    pub channel_type: String,
    pub sender_channel: u32,
    pub initial_window: u32,
    pub max_packet: u32,
}

/// Parse SSH_MSG_CHANNEL_OPEN_CONFIRMATION.
pub fn parse_channel_open_confirmation(payload: &[u8]) -> Option<(u32, u32, u32, u32)> {
    if payload.is_empty() || payload[0] != packet::msg::CHANNEL_OPEN_CONFIRMATION {
        return None;
    }
    let (recipient_channel, offset) = packet::decode_u32(payload, 1)?;
    let (sender_channel, offset) = packet::decode_u32(payload, offset)?;
    let (initial_window, offset) = packet::decode_u32(payload, offset)?;
    let (max_packet, _) = packet::decode_u32(payload, offset)?;
    Some((recipient_channel, sender_channel, initial_window, max_packet))
}

/// Parse SSH_MSG_CHANNEL_REQUEST.
pub fn parse_channel_request(payload: &[u8]) -> Option<ChannelRequest> {
    if payload.is_empty() || payload[0] != packet::msg::CHANNEL_REQUEST {
        return None;
    }
    let (recipient_channel, offset) = packet::decode_u32(payload, 1)?;
    let (request_type, offset) = packet::decode_string(payload, offset)?;
    let request_type = core::str::from_utf8(request_type).ok()?;
    let want_reply = if offset < payload.len() { payload[offset] != 0 } else { false };
    let data_offset = offset + 1;

    match request_type {
        "pty-req" => {
            let (term, off) = packet::decode_string(payload, data_offset)?;
            let term = core::str::from_utf8(term).ok()?.to_string();
            let (width_chars, off) = packet::decode_u32(payload, off)?;
            let (height_rows, off) = packet::decode_u32(payload, off)?;
            let (width_pixels, off) = packet::decode_u32(payload, off)?;
            let (height_pixels, _) = packet::decode_u32(payload, off)?;

            Some(ChannelRequest::PtyReq {
                recipient_channel,
                want_reply,
                term,
                width_chars,
                height_rows,
                width_pixels,
                height_pixels,
            })
        }
        "shell" => {
            Some(ChannelRequest::Shell {
                recipient_channel,
                want_reply,
            })
        }
        "exec" => {
            let (command, _) = packet::decode_string(payload, data_offset)?;
            let command = core::str::from_utf8(command).ok()?.to_string();
            Some(ChannelRequest::Exec {
                recipient_channel,
                want_reply,
                command,
            })
        }
        "window-change" => {
            let (width_chars, off) = packet::decode_u32(payload, data_offset)?;
            let (height_rows, _) = packet::decode_u32(payload, off)?;
            Some(ChannelRequest::WindowChange {
                recipient_channel,
                width_chars,
                height_rows,
            })
        }
        _ => None,
    }
}

pub enum ChannelRequest {
    PtyReq {
        recipient_channel: u32,
        want_reply: bool,
        term: String,
        width_chars: u32,
        height_rows: u32,
        width_pixels: u32,
        height_pixels: u32,
    },
    Shell {
        recipient_channel: u32,
        want_reply: bool,
    },
    Exec {
        recipient_channel: u32,
        want_reply: bool,
        command: String,
    },
    WindowChange {
        recipient_channel: u32,
        width_chars: u32,
        height_rows: u32,
    },
}

/// Parse SSH_MSG_CHANNEL_DATA.
pub fn parse_channel_data(payload: &[u8]) -> Option<(u32, &[u8])> {
    if payload.is_empty() || payload[0] != packet::msg::CHANNEL_DATA {
        return None;
    }
    let (channel_id, offset) = packet::decode_u32(payload, 1)?;
    let (data, _) = packet::decode_string(payload, offset)?;
    Some((channel_id, data))
}

/// Parse SSH_MSG_CHANNEL_WINDOW_ADJUST.
pub fn parse_window_adjust(payload: &[u8]) -> Option<(u32, u32)> {
    if payload.is_empty() || payload[0] != packet::msg::CHANNEL_WINDOW_ADJUST {
        return None;
    }
    let (channel_id, offset) = packet::decode_u32(payload, 1)?;
    let (bytes_to_add, _) = packet::decode_u32(payload, offset)?;
    Some((channel_id, bytes_to_add))
}
