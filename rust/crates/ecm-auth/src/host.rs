//! Adapters for programs on a computer (feature `host`): a TCP [`Stream`] over the
//! socket ABI (wasm32 only) and the accounts folder ([`Files`]) on the computer's
//! storage (std::fs, with the computer's own calls as the fallback on wasm32). The rest
//! of the crate stays plain logic.

#[cfg(target_arch = "wasm32")]
use ecm_host_abi::socket::{self, PollFd, SockAddrIn, AF_INET, MSG_DONTWAIT, POLLERR, POLLHUP, POLLIN, SOCK_STREAM};

use crate::client::Stream;
use crate::store::Files;
use crate::AuthError;

/// A connection to the realm at `host` (`host:port`, port 7600 by default). Off a
/// computer (host tests) there is no network: an error.
pub fn connect(host: &str) -> Result<Box<dyn Stream>, AuthError> {
    #[cfg(target_arch = "wasm32")]
    {
        Ok(Box::new(SocketStream::connect(host)?))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::host_port(host)?;
        Err(AuthError::Network("auth needs a computer's network".into()))
    }
}

/// A TCP connection to a realm.
#[cfg(target_arch = "wasm32")]
pub struct SocketStream {
    fd: i32,
}

#[cfg(target_arch = "wasm32")]
impl SocketStream {
    /// Connect to `host` or `host:port` (port 7600 when none is given).
    pub fn connect(host: &str) -> Result<SocketStream, AuthError> {
        let (h, port) = crate::host_port(host)?;
        let ip = socket::resolve(&h).map_err(|e| AuthError::Network(format!("{h}: {}", e.message())))?;
        let fd = socket::socket(AF_INET, SOCK_STREAM, 0);
        if fd < 0 {
            return Err(AuthError::Network("no socket".into()));
        }
        if socket::connect(fd, &SockAddrIn::from_ip_port(ip, port)) != 0 {
            socket::close(fd);
            return Err(AuthError::Network(format!("can't connect to {h}:{port}")));
        }
        Ok(SocketStream { fd })
    }
}

#[cfg(target_arch = "wasm32")]
impl Stream for SocketStream {
    fn write_all(&mut self, mut data: &[u8]) -> Result<(), AuthError> {
        while !data.is_empty() {
            let n = socket::send(self.fd, &data[..data.len().min(1400)], 0);
            if n <= 0 {
                return Err(AuthError::Closed);
            }
            data = &data[n as usize..];
        }
        Ok(())
    }

    fn read_exact(&mut self, buf: &mut [u8], timeout_ms: u32) -> Result<(), AuthError> {
        let mut got = 0;
        while got < buf.len() {
            let mut fds = [PollFd { fd: self.fd, events: POLLIN, revents: 0 }];
            if socket::poll(&mut fds, timeout_ms as i32) <= 0 {
                return Err(AuthError::Timeout);
            }
            if fds[0].revents & (POLLIN | POLLERR | POLLHUP) == 0 {
                continue;
            }
            match socket::recv(self.fd, &mut buf[got..], MSG_DONTWAIT) {
                n if n > 0 => got += n as usize,
                -2 => {}
                _ => return Err(AuthError::Closed),
            }
        }
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
impl Drop for SocketStream {
    fn drop(&mut self) {
        socket::close(self.fd);
    }
}

/// An accounts folder on the computer (`/var/apps/<app>/accounts`, a script's `.auth`).
pub struct DirFiles {
    dir: String,
}

impl DirFiles {
    pub fn new(dir: &str) -> DirFiles {
        DirFiles { dir: dir.trim_end_matches('/').to_string() }
    }

    fn path(&self, name: &str) -> String {
        format!("{}/{name}", self.dir)
    }
}

impl Files for DirFiles {
    fn read(&self, name: &str) -> Option<String> {
        let p = self.path(name);
        let text = std::fs::read_to_string(&p).ok();
        #[cfg(target_arch = "wasm32")]
        let text = text.or_else(|| ecm_host_abi::fs::read_file_bytes(p.trim_start_matches('/')).map(|b| String::from_utf8_lossy(&b).into_owned()));
        text
    }

    fn write(&mut self, name: &str, text: &str) -> Result<(), AuthError> {
        let p = self.path(name);
        if std::fs::write(&p, text).is_ok() {
            return Ok(());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::fs::create_dir_all(&self.dir).and_then(|_| std::fs::write(&p, text)).map_err(|e| AuthError::Storage(format!("{p}: {e}")))
        }
        // std::fs::create_dir_all fails on these computers: make each level with the
        // computer's own call, then write.
        #[cfg(target_arch = "wasm32")]
        {
        let rel = self.dir.trim_start_matches('/');
        let mut dir = String::new();
        for part in rel.split('/').filter(|p| !p.is_empty() && *p != ".") {
            if !dir.is_empty() {
                dir.push('/');
            }
            dir.push_str(part);
            ecm_host_abi::fs::mkdir(&dir);
        }
        if std::fs::write(&p, text).is_ok() || ecm_host_abi::fs::write_file(p.trim_start_matches('/'), text) >= 0 {
            Ok(())
        } else {
            Err(AuthError::Storage(format!("can't write {p}")))
        }
        }
    }

    fn remove(&mut self, name: &str) {
        let p = self.path(name);
        if std::fs::remove_file(&p).is_err() {
            #[cfg(target_arch = "wasm32")]
            ecm_host_abi::fs::delete(p.trim_start_matches('/'));
        }
    }
}

/// The scripting `auth` module (Python and Lua call the same functions, so they behave
/// the same): logins and pins live in `dir` (a script's `.auth/` folder), hosts are
/// `host[:port]`, and the pinning rules are the apps' ([`crate::store`]).
pub mod script {
    use super::{connect, DirFiles};
    use crate::store::{AccountStore, Login, Pin};
    use crate::{endpoint, hex, parse_fingerprint, unhex_n, AuthError};

    /// A relative folder counts from the current directory.
    fn store(dir: &str) -> AccountStore<DirFiles> {
        let dir = if dir.starts_with('/') || std::path::Path::new(dir).is_absolute() {
            dir.to_string()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => format!("{}/{}", cwd.to_string_lossy().trim_end_matches('/'), dir.trim_start_matches("./")),
                Err(_) => dir.to_string(),
            }
        };
        AccountStore::new(DirFiles::new(&dir))
    }

    pub fn signin(dir: &str, host: &str, user: &str, password: &str) -> Result<Login, AuthError> {
        store(dir).sign_in(connect(host)?, host, user, password)
    }

    pub fn signup(dir: &str, host: &str, user: &str, password: &str, display: Option<&str>) -> Result<Login, AuthError> {
        store(dir).sign_up(connect(host)?, host, user, password, display)
    }

    /// The saved login for `host`, or for the realm signed in to last.
    fn login(dir: &str, host: Option<&str>) -> Result<Login, AuthError> {
        store(dir).login_for(host)
    }

    /// A token for the backend `audience` (its key fingerprint, any form).
    pub fn token(dir: &str, audience: &str, host: Option<&str>) -> Result<String, AuthError> {
        let l = login(dir, host)?;
        store(dir).token(connect(&l.host)?, &l.realm_fp, audience).map(|(t, _)| t)
    }

    /// The proof (hex) for a backend's connect nonce (hex), with the session key of that login.
    pub fn prove(dir: &str, nonce: &str, audience: &str, host: Option<&str>) -> Result<String, AuthError> {
        let l = login(dir, host)?;
        let nonce: [u8; 32] = unhex_n(nonce.trim()).ok_or_else(|| AuthError::Protocol("the nonce must be 64 hex digits".into()))?;
        store(dir).prove(&l.realm_fp, &nonce, audience).map(|p| hex(&p))
    }

    /// The realm at `host`: its fingerprint (and whether it is pinned), without signing in.
    pub fn fingerprint(dir: &str, host: &str) -> Result<(String, String, bool), AuthError> {
        let (info, check) = store(dir).fingerprint(connect(host)?, host)?;
        Ok((info.realm_fp, info.realm_name, check == crate::PinCheck::Pinned))
    }

    /// Sign out (revoking the refresh token if the realm answers). False if not signed in.
    pub fn logout(dir: &str, host: Option<&str>) -> Result<bool, AuthError> {
        let Ok(l) = login(dir, host) else { return Ok(false) };
        let stream = connect(&l.host).ok();
        store(dir).logout(stream, &l.realm_fp)?;
        Ok(true)
    }

    pub fn pins(dir: &str) -> Vec<Pin> {
        store(dir).pins().pins
    }

    /// Remove a pin by hand: by fingerprint (any form) or by host. False if none matched.
    pub fn unpin(dir: &str, which: &str) -> Result<bool, AuthError> {
        let mut s = store(dir);
        let fp = match parse_fingerprint(which) {
            Some(fp) => fp,
            None => match endpoint(which).ok().and_then(|h| s.pins().pins.into_iter().find(|p| p.host == h)) {
                Some(p) => p.fp,
                None => return Ok(false),
            },
        };
        s.remove_pin(&fp)
    }
}
