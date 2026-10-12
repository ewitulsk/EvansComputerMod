//! How the system picker and chooser answer the compositor: one line written
//! to `/dev/uictl` (docs/gui/APPS.md "File picker and chooser").
//!
//! ```text
//! picked <token> <absolute path>      the player picked this file
//! picked <token> cancel               the player cancelled
//! chosen <token> <app id> [always]    open the file with this app (always: make it the default)
//! chosen <token> cancel
//! ```
//!
//! The compositor starts the program with `--req <token>` and only accepts the
//! answer from that program's pid, once.

/// The line for a picker answer (`None` = cancelled).
pub fn picked_line(token: &str, path: Option<&str>) -> String {
    format!("picked {} {}", token, path.unwrap_or("cancel"))
}

/// The line for a chooser answer (`None` = cancelled).
pub fn chosen_line(token: &str, app: Option<&str>, always: bool) -> String {
    match app {
        Some(id) if always => format!("chosen {token} {id} always"),
        Some(id) => format!("chosen {token} {id}"),
        None => format!("chosen {token} cancel"),
    }
}

/// Where answers go.
pub trait Answer {
    /// Send one line; `Ok` is the compositor's reply.
    fn send(&mut self, line: &str) -> Result<String, String>;
}

/// `/dev/uictl` on a computer.
pub struct UiCtl;

impl Answer for UiCtl {
    fn send(&mut self, line: &str) -> Result<String, String> {
        use std::io::{Read, Write};
        let mut dev = std::fs::OpenOptions::new().read(true).write(true).open("/dev/uictl").map_err(|e| format!("no GUI device ({e})"))?;
        dev.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
        let mut answer = String::new();
        let _ = dev.read_to_string(&mut answer);
        // "ok" when accepted; "gui: ..." says why not.
        if answer.starts_with("gui: ") {
            return Err(answer.trim().to_string());
        }
        Ok(answer)
    }
}

/// Answers kept in memory (tests).
#[derive(Default, Clone, Debug)]
pub struct Recorded(pub Vec<String>);

impl Answer for Recorded {
    fn send(&mut self, line: &str) -> Result<String, String> {
        self.0.push(line.to_string());
        Ok("ok\n".into())
    }
}

/// `--name value` from program arguments.
pub fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines() {
        assert_eq!(picked_line("a1", Some("/home/my notes.txt")), "picked a1 /home/my notes.txt");
        assert_eq!(picked_line("a1", None), "picked a1 cancel");
        assert_eq!(chosen_line("b2", Some("dev.x"), true), "chosen b2 dev.x always");
        assert_eq!(chosen_line("b2", Some("dev.x"), false), "chosen b2 dev.x");
        assert_eq!(chosen_line("b2", None, true), "chosen b2 cancel");
        let a = vec!["--req".to_string(), "t".into(), "--mode".into()];
        assert_eq!(arg(&a, "--req").as_deref(), Some("t"));
        assert_eq!(arg(&a, "--mode"), None);
    }
}
