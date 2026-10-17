//! The account side of `chat`, without host calls (so it is tested on the host): the
//! extra arguments, where the logins live, and the words shown to the player.

use ecm_auth::AuthError;

pub const USAGE: &str = "Usage: chat [--user NAME [--new]] [server[:port]]\n\
       chat --forget FINGERPRINT\n\
  Talk in a Discord backend's home server (#general) from the console. With no\n\
  server: /etc/chat.conf, else the network (disco).\n\
  Accounts are required: --user NAME signs in (the password is asked for and not\n\
  shown); --new makes the account first. The login is kept in ~/.chat/accounts,\n\
  so next time plain 'chat' signs in by itself. --forget removes a realm's pin\n\
  (and its saved login) when its key changed for a reason you know.\n\
  While chatting: /who  list users    /help  this help    /quit  leave";

/// What `chat` takes beyond the old arguments.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AccountArgs {
    pub user: Option<String>,
    pub forget: Option<String>,
    /// Make the account (sign up) instead of signing in.
    pub new: bool,
    pub help: bool,
}

/// Take `--user NAME`, `--new`, `--forget FP` (and `-h`) out of the arguments; the rest
/// goes to the old parser (server, port).
pub fn account_args(all: &[String]) -> Result<(AccountArgs, Vec<String>), String> {
    let mut a = AccountArgs::default();
    let mut rest = Vec::new();
    let mut i = 0;
    while i < all.len() {
        match all[i].as_str() {
            "--user" | "-u" => {
                i += 1;
                let u = all.get(i).ok_or("--user needs a username")?;
                a.user = Some(ecm_auth::username(u).map_err(|e| e.to_string())?);
            }
            "--forget" => {
                i += 1;
                let f = all.get(i).ok_or("--forget needs a fingerprint")?;
                a.forget = Some(ecm_auth::parse_fingerprint(f).ok_or_else(|| format!("'{f}' is not a fingerprint (32 hex digits)"))?);
            }
            "--new" => a.new = true,
            "-h" | "--help" => a.help = true,
            "--nick" | "-n" => return Err("chat shows your account's username now: use --user NAME".into()),
            other => rest.push(other.to_string()),
        }
        i += 1;
    }
    if a.new && a.user.is_none() {
        return Err("--new needs --user NAME".into());
    }
    Ok((a, rest))
}

/// Where chat keeps its pins and logins: `<home>/.chat/accounts`.
pub fn accounts_dir(home: Option<&str>) -> String {
    let home = home.filter(|h| !h.is_empty()).unwrap_or("/home").trim_end_matches('/');
    format!("{home}/.chat/accounts")
}

/// A typed answer to "[y/N]".
pub fn yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// The first-contact question.
pub fn first_contact(realm_name: &str, host: &str, fp: &str) -> String {
    format!(
        "First sign-in at \"{realm_name}\" ({host}).\nIts fingerprint is {}.\nTrust it and sign in? [y/N] ",
        ecm_auth::display_fingerprint(fp)
    )
}

/// What to print for a refusal (a changed realm also says how to remove the pin).
pub fn explain(e: &AuthError) -> String {
    match e {
        AuthError::RealmChanged { old_fp, .. } => format!(
            "chat: {e}\nIf you know why its key changed, remove the old pin with: chat --forget {old_fp}"
        ),
        AuthError::NotSignedIn(_) | AuthError::SignInExpired => format!("chat: {e} (chat --user NAME)"),
        e => format!("chat: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn account_arguments_come_out_and_the_rest_stays() {
        let (a, rest) = account_args(&s(&["--user", "Alice", "10.0.0.5:7790"])).unwrap();
        assert_eq!(a, AccountArgs { user: Some("alice".into()), ..Default::default() });
        assert_eq!(rest, ["10.0.0.5:7790"]);
        let (a, rest) = account_args(&s(&["--forget", "3F:A1:9C:0D:00:00:00:00:00:00:00:00:00:00:BE:EF"])).unwrap();
        assert_eq!(a.forget.as_deref(), Some("3fa19c0d00000000000000000000beef"));
        assert!(rest.is_empty());
        assert!(account_args(&s(&["--user", "u", "--new"])).unwrap().0.new);
        assert!(account_args(&s(&["--new"])).is_err());
        assert!(account_args(&s(&["--user", "bad name!"])).is_err());
        assert!(account_args(&s(&["--forget", "xyz"])).is_err());
        assert!(account_args(&s(&["--nick", "old"])).unwrap_err().contains("--user"));
        assert!(account_args(&s(&["-h"])).unwrap().0.help);
    }

    #[test]
    fn words_for_the_player() {
        assert_eq!(accounts_dir(Some("/home/alex/")), "/home/alex/.chat/accounts");
        assert_eq!(accounts_dir(None), "/home/.chat/accounts");
        assert!(yes(" Y ") && yes("yes") && !yes("") && !yes("n"));
        let q = first_contact("Tech Village", "10.0.8.1:7600", "3fa19c0d00000000000000000000beef");
        assert!(q.contains("\"Tech Village\" (10.0.8.1:7600)") && q.contains("3F:A1:9C:0D:") && q.ends_with("[y/N] "));
        let e = AuthError::RealmChanged { name: "Tech Village".into(), host: "h".into(), old_fp: "a".repeat(32), new_fp: "b".repeat(32) };
        let t = explain(&e);
        assert!(t.contains("The identity") || t.contains("the identity of Tech Village has changed"), "{t}");
        assert!(t.ends_with(&format!("chat --forget {}", "a".repeat(32))));
        assert_eq!(explain(&AuthError::WrongPassword), "chat: wrong username or password");
    }
}
