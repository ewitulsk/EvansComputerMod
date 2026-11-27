//! `repo-api admin <verb> ...`: the admin operations, run on the server against the
//! same database and blob store as the service.

use crate::{ops, signing_key, App, Config};
use backend_common::ApiError;

pub const USAGE: &str = "\
usage: repo-api admin <verb> ...
  promote <id> <version> <channel>   put a published version into a channel
  demote <id> <version> <channel>    take it out of a channel
  takedown <id> [<version>]          remove a version, or the whole package, with its files
  channel <name> [<title>]           create a channel
  reports                            list reports, newest first
  pubkey                             print the repository's public key (from REPO_SIGNING_KEY)
  keygen                             print a new REPO_SIGNING_KEY and its public key
";

/// Run an admin command; the text to print, or the error.
pub async fn run(args: &[String]) -> Result<String, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    // These two need no database.
    match words.as_slice() {
        ["keygen"] => {
            let (public, seed) = ecm_pkg::sign::generate();
            return Ok(format!("REPO_SIGNING_KEY={seed}\n# public key (pkg repo add ... --key): {public}\n"));
        }
        ["pubkey"] => {
            let seed = backend_common::env::required("REPO_SIGNING_KEY")?;
            return Ok(format!("{}\n", signing_key(&seed)?.1));
        }
        [] | ["help"] | ["--help"] | ["-h"] => return Ok(USAGE.to_string()),
        _ => {}
    }
    let app = crate::build(Config::from_env()?)?;
    run_on(&app, &words).await
}

/// The verbs that work on a repository.
pub async fn run_on(app: &App, words: &[&str]) -> Result<String, String> {
    let said = |e: ApiError| e.message;
    match words {
        ["promote", id, version, channel] => {
            ops::promote(app, id, version, channel).map_err(said)?;
            Ok(format!("{id} {version} is in {channel}\n"))
        }
        ["demote", id, version, channel] => {
            ops::demote(app, id, version, channel).map_err(said)?;
            Ok(format!("{id} {version} is not in {channel}\n"))
        }
        ["takedown", id] => {
            ops::takedown(app, id, None).await.map_err(said)?;
            Ok(format!("{id} was taken down\n"))
        }
        ["takedown", id, version] => {
            ops::takedown(app, id, Some(version)).await.map_err(said)?;
            Ok(format!("{id} {version} was taken down\n"))
        }
        ["channel" | "channels", name] => {
            ops::create_channel(app, name, name).map_err(said)?;
            Ok(format!("created the channel {name}\n"))
        }
        ["channel" | "channels", name, title @ ..] => {
            ops::create_channel(app, name, &title.join(" ")).map_err(said)?;
            Ok(format!("created the channel {name}\n"))
        }
        ["reports"] => {
            let mut out = String::new();
            for r in ops::reports(app).map_err(said)? {
                let text = |k: &str| r[k].as_str().unwrap_or("").to_string();
                out.push_str(&format!("#{} {} {} by {}: {}\n", r["report"], text("created"), text("id"), text("reporter"), text("reason")));
            }
            if out.is_empty() {
                out.push_str("no reports\n");
            }
            Ok(out)
        }
        _ => Err(USAGE.to_string()),
    }
}
