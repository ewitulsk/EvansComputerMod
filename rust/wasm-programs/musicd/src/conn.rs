//! One HTTP/1.1 client connection: requests in (pipelined, with bodies),
//! responses out, file bodies read a chunk at a time as the socket takes
//! them. The loop sends only when the socket can take more (POLLOUT), so a
//! slow listener holds its own connection back and nobody else's.

use crate::api::{App, Body, ConnCtx, Reply};
use crate::disk::Disk;
use ecm_music::http::{self, HttpError};

/// Bytes read from a file (and offered to the socket) at a time.
pub const CHUNK: usize = 16 * 1024;
/// Largest request body (playlist edits are small).
pub const MAX_BODY: usize = 64 * 1024;

pub struct Conn {
    inbuf: Vec<u8>,
    out: Vec<u8>,
    out_pos: usize,
    /// The rest of a file body: path, next offset, end.
    file: Option<(String, u64, u64)>,
    pub ctx: ConnCtx,
    /// Close once everything queued is sent.
    close_after: bool,
    pub last_ms: u64,
    /// Requests answered (for the log and tests).
    pub requests: u64,
}

impl Conn {
    pub fn new(peer: &str, now: u64) -> Conn {
        let ip = peer.rsplit_once(':').map_or(peer, |(ip, _)| ip);
        Conn { inbuf: Vec::new(), out: Vec::new(), out_pos: 0, file: None, ctx: ConnCtx::new(ip), close_after: false, last_ms: now, requests: 0 }
    }

    /// Nothing queued to send.
    pub fn idle(&self) -> bool {
        self.out_pos >= self.out.len() && self.file.is_none()
    }

    pub fn wants_write(&self) -> bool {
        !self.idle()
    }

    /// Everything is sent and the connection should close.
    pub fn finished(&self) -> bool {
        self.close_after && self.idle()
    }

    /// Bytes received from the client.
    pub fn receive<D: Disk>(&mut self, app: &mut App<D>, data: &[u8], now: u64) {
        self.last_ms = now;
        if self.close_after {
            return;
        }
        self.inbuf.extend_from_slice(data);
        self.process(app, now);
    }

    /// Answer the next waiting request(s) while nothing is queued (responses stay in order).
    pub fn process<D: Disk>(&mut self, app: &mut App<D>, now: u64) {
        while self.idle() && !self.close_after {
            let (req, used) = match http::parse_request(&self.inbuf) {
                Ok(Some(r)) => r,
                Ok(None) => {
                    if self.inbuf.len() > http::MAX_HEAD {
                        self.fail(431, "the request head is too large");
                    }
                    return;
                }
                Err(HttpError::TooLarge) => return self.fail(431, "the request head is too large"),
                Err(HttpError::Bad(why)) => return self.fail(400, why),
            };
            let len = req.content_length().unwrap_or(0);
            if len > MAX_BODY {
                return self.fail(413, "the request body is too large");
            }
            if self.inbuf.len() < used + len {
                return; // the body is still coming
            }
            let body: Vec<u8> = self.inbuf[used..used + len].to_vec();
            self.inbuf.drain(..used + len);
            let reply = app.handle(&req, &body, &mut self.ctx, now);
            self.requests += 1;
            self.queue(reply, req.method == "HEAD", req.keep_alive());
        }
    }

    fn fail(&mut self, status: u16, why: &str) {
        self.queue(Reply::error(status, "bad_request", why), false, false);
        self.inbuf.clear();
    }

    fn queue(&mut self, reply: Reply, head_only: bool, keep_alive: bool) {
        self.out.clear();
        self.out_pos = 0;
        self.out.extend_from_slice(&reply.head);
        if !head_only {
            match reply.body {
                Body::Bytes(b) => self.out.extend_from_slice(&b),
                Body::File { path, start, len } => {
                    if len > 0 {
                        self.file = Some((path, start, start + len));
                    }
                }
            }
        }
        if !keep_alive {
            self.close_after = true;
        }
    }

    /// The next bytes to send (reading the next file chunk when needed).
    pub fn pending<D: Disk>(&mut self, disk: &D) -> &[u8] {
        if self.out_pos >= self.out.len() {
            self.out.clear();
            self.out_pos = 0;
            if let Some((path, pos, end)) = self.file.take() {
                let n = ((end - pos) as usize).min(CHUNK);
                match disk.read_range(&path, pos, n) {
                    Some(d) if !d.is_empty() => {
                        let next = pos + d.len() as u64;
                        self.out = d;
                        if next < end {
                            self.file = Some((path, next, end));
                        }
                    }
                    // The file shrank or vanished mid-stream: the length promised can't be
                    // kept, so the connection closes (the client resumes with a Range).
                    _ => self.close_after = true,
                }
            }
        }
        &self.out[self.out_pos..]
    }

    /// `n` bytes of [`Conn::pending`] were sent.
    pub fn sent(&mut self, n: usize) {
        self.out_pos = (self.out_pos + n).min(self.out.len());
    }

    /// Drain everything queued (tests: an always-ready socket).
    pub fn drain_all<D: Disk>(&mut self, disk: &D) -> Vec<u8> {
        let mut all = Vec::new();
        loop {
            let p = self.pending(disk).to_vec();
            if p.is_empty() {
                break;
            }
            self.sent(p.len());
            all.extend(p);
        }
        all
    }
}
