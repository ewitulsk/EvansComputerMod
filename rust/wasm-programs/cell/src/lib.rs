//! `cell`: the cellular command (like `mmcli` / `nmcli radio wwan`).
//!
//! ```text
//! cell                 status: state, SIM, carrier, cell id, signal bars, SINR, CQI, address, traffic, handovers
//! cell scan            cells heard (carrier, cell, profile, channel, signal)
//! cell up | down       attach / detach (airplane mode for the modem)
//! cell roaming on|off  data roaming (saved in /etc/celld.toml)
//! ```
//!
//! It talks to `celld` (Cellular v2) over its control socket. On a computer
//! whose modem still runs the v1 engine (`lte_modem` / `mmwave_modem`
//! peripherals: the server config `modem.framedRadio = false`) it shows the v1
//! modem pages as before (phase 4 removes that path).

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    Status,
    Scan,
    Up,
    Down,
    Roaming(bool),
}

pub const USAGE: &str = "usage: cell [status|scan|up|down|roaming on|off]";

pub fn parse(args: &[String]) -> Result<Cmd, String> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    Ok(match a.as_slice() {
        [] | ["status"] => Cmd::Status,
        ["scan"] | ["cells"] => Cmd::Scan,
        ["up"] => Cmd::Up,
        ["down"] => Cmd::Down,
        ["roaming", "on"] => Cmd::Roaming(true),
        ["roaming", "off"] => Cmd::Roaming(false),
        _ => return Err(USAGE.into()),
    })
}

/// The control-socket request for a command.
pub fn request(c: &Cmd) -> &'static str {
    match c {
        Cmd::Status => "status",
        Cmd::Scan => "cells",
        Cmd::Up => "up",
        Cmd::Down => "down",
        Cmd::Roaming(true) => "roaming on",
        Cmd::Roaming(false) => "roaming off",
    }
}

/// What `cell` prints for celld's answer to `c`.
pub fn render(c: &Cmd, answer: &str) -> String {
    match c {
        Cmd::Status => celld::ctl::render_status(&celld::ctl::Status::parse(answer)),
        Cmd::Scan => celld::ctl::render_cells(&celld::ctl::parse_cells(answer)),
        _ => match answer.trim().strip_prefix("ok ") {
            Some(rest) => format!("cell: {rest}\n"),
            None => format!("cell: {}\n", answer.trim()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn commands() {
        assert_eq!(parse(&a("")).unwrap(), Cmd::Status);
        assert_eq!(parse(&a("scan")).unwrap(), Cmd::Scan);
        assert_eq!(parse(&a("roaming off")).unwrap(), Cmd::Roaming(false));
        assert_eq!(request(&parse(&a("up")).unwrap()), "up");
        assert!(parse(&a("roaming maybe")).is_err());
        assert!(parse(&a("fly")).is_err());
    }

    #[test]
    fn renders_answers() {
        let st = "state connected\nmessage on cell-red cell 1\nmodem module_1\nprofile lte\nsim_carrier cell-red\nimsi 310150000000001\ncarrier cell-red\ncell 1\nchannel 1\nrssi -60.0\nsinr 30.0\ncqi 15\nbars 4\nip 100.65.2.2/24\ngateway 100.65.2.1\ndns -\nhandovers 0\n";
        let out = render(&Cmd::Status, st);
        assert!(out.contains("state     connected - on cell-red cell 1"), "{out}");
        assert!(out.contains("SIM       cell-red, IMSI 310150000000001"), "{out}");
        assert!(out.contains("[****]"), "{out}");
        assert_eq!(render(&Cmd::Down, "ok detaching\n"), "cell: detaching\n");
        assert_eq!(render(&Cmd::Up, "error nope\n"), "cell: error nope\n");
    }
}
