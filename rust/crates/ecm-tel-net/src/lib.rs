//! The telephony programs' computer side (docs/TELEPHONY.md).
//!
//! | Module | What |
//! |---|---|
//! | [`http`] | HTTP/1.0 requests and responses with binary bodies (MMS, visual voicemail) |
//! | [`smtp`] | SMTP for MM4: the server as a line machine, the client's dialogue |
//! | `sys` | (WASI) sockets, the HSS client, ENUM / SRV / MX, the HTTP server and client, SMTP sending, logs, CDRs |

pub mod http;
pub mod smtp;
#[cfg(target_os = "wasi")]
pub mod sys;
