//! SSH session management — ties together channels and processes.

use super::channel::ChannelManager;
use super::transport::SshTransport;
use super::auth::UserDb;

/// An SSH server session.
pub struct SshSession {
    pub transport: SshTransport,
    pub channels: ChannelManager,
    pub user_db: UserDb,
    pub authenticated: bool,
    pub username: String,
}

impl SshSession {
    pub fn new_server() -> Self {
        Self {
            transport: SshTransport::new(true),
            channels: ChannelManager::new(),
            user_db: UserDb::load(),
            authenticated: false,
            username: String::new(),
        }
    }

    pub fn new_client() -> Self {
        Self {
            transport: SshTransport::new(false),
            channels: ChannelManager::new(),
            user_db: UserDb::load(),
            authenticated: false,
            username: String::new(),
        }
    }
}
