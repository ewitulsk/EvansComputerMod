//! `telephonyd`'s message and call-log store (the daemon is `prog.rs`).

pub mod store;

/// A file a client names (`play`, `record`, `mms` parts, `voicemail-fetch`): telephonyd
/// reads and writes it with its own rights, so only the user's files qualify —
/// under `/home/` or `/tmp/`, not the address book, no `..` — or an app with the
/// `phone` or `sms` permission could read or overwrite anything through it.
pub fn user_path(p: &str) -> Result<String, String> {
    let ok_root = p.starts_with("/home/") || p.starts_with("/tmp/");
    let bad = p.split('/').any(|c| c == "..") || p.starts_with("/home/.contacts");
    if ok_root && !bad {
        Ok(p.to_string())
    } else {
        Err(format!("{p}: telephonyd only uses files under /home/ or /tmp/"))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_the_users_files() {
        assert!(super::user_path("/home/Pictures/a.png").is_ok());
        assert!(super::user_path("/tmp/vm-3.wav").is_ok());
        assert!(super::user_path("/var/telephony/messages.log").is_err());
        assert!(super::user_path("/home/../etc/cored.db").is_err());
        assert!(super::user_path("/home/.contacts/contacts.vcf").is_err());
        assert!(super::user_path("/etc/passwd").is_err());
    }
}
