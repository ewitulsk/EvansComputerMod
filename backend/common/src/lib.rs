//! Shared pieces of `auth-api` and `repo-api` (docs/REPO_API.md).

pub mod db;
pub mod env;
pub mod error;
pub mod html;
pub mod http;
pub mod ratelimit;
pub mod token;

pub use error::ApiError;

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// Seconds since the Unix epoch.
pub fn now() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp()
}

/// An RFC 3339 UTC string (`2026-01-02T03:04:05Z`) for a Unix time.
pub fn rfc3339(unix: i64) -> String {
    OffsetDateTime::from_unix_timestamp(unix).ok().and_then(|t| t.format(&Rfc3339).ok()).unwrap_or_default()
}

/// Lower-case hex.
pub fn hex(b: &[u8]) -> String {
    const D: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push(D[(x >> 4) as usize] as char);
        s.push(D[(x & 15) as usize] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    #[test]
    fn times_and_hex() {
        assert_eq!(super::rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(super::rfc3339(1_767_323_045), "2026-01-02T03:04:05Z");
        assert_eq!(super::hex(&[0, 0xab, 0xff]), "00abff");
    }
}
