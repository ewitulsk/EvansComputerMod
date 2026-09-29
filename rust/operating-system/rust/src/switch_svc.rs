//! The L2 switch as a kernel service.
//!
//! The switch runs independently of the shell: `switch on` starts it in the
//! background, `switch` opens its CLI, and neither Ctrl+T nor leaving the
//! CLI of a background switch stops it. Forwarding and protocols live in the
//! pure `ecm-bridge` crate; this module owns lifecycle, persistence
//! (`/switch.cfg`) and applying CLI side effects (SVI addresses, save).

use ecm_bridge::cli::{self, CliEffect, CliSession};
use ecm_bridge::Bridge;
use ecm_net::types::MacAddr;

use crate::console::Term;
use crate::fs;
use crate::net::Net;

pub const CONFIG_PATH: &str = "switch.cfg";

pub struct SwitchService {
    /// Present while the CLI is open.
    session: Option<CliSession>,
    /// Keep forwarding after the CLI exits (`switch on` / CLI `on`).
    persist: bool,
}

pub enum CliState {
    Open,
    Closed,
}

impl SwitchService {
    pub fn new() -> Self {
        Self { session: None, persist: false }
    }

    pub fn running(net: &Net) -> bool {
        net.bridge().is_some()
    }

    pub fn cli_open(&self) -> bool {
        self.session.is_some()
    }

    pub fn prompt(&self) -> String {
        self.session.as_ref().map(cli::prompt).unwrap_or_default()
    }

    fn start(&mut self, net: &mut Net, con: &mut dyn Term, now: i64) -> bool {
        if net.port_count() == 0 {
            con.println("switch: this computer has no network interfaces");
            return false;
        }
        let macs = net.port_macs();
        // A locally administered bridge MAC derived from port 0 so it never
        // collides with a port MAC.
        let mut bm = macs[0].0;
        bm[0] = (bm[0] | 0x02) ^ 0x04;
        let bridge = Bridge::new(&macs, MacAddr(bm), now);
        net.attach_bridge(bridge, now);
        if let Some(text) = fs::read_to_string(CONFIG_PATH) {
            // One session for the whole file so block contexts
            // (`interface ethN` ... `exit`) replay correctly.
            if let Some(bridge) = net.bridge_mut() {
                let (_, r) = cli::load_config(bridge, &text, now);
                if !r.output.is_empty() {
                    con.print(&r.output);
                }
                for e in r.effects {
                    Self::apply_config_effect(net, e, now);
                }
            }
            net.sync_bridge_ports();
        }
        true
    }

    fn apply_config_effect(net: &mut Net, e: CliEffect, now: i64) {
        match e {
            CliEffect::SviAddress { vlan, ip, prefix } => {
                net.set_svi(vlan, ip, prefix, now);
            }
            CliEffect::SviRemove { vlan } => net.remove_svi(vlan),
            _ => {}
        }
    }

    pub fn stop(&mut self, net: &mut Net) {
        self.session = None;
        self.persist = false;
        net.detach_bridge();
    }

    /// `switch`: open the CLI, starting the switch if needed.
    pub fn enter_cli(&mut self, net: &mut Net, con: &mut dyn Term, now: i64) -> bool {
        if Self::running(net) {
            con.println("Entering switch configuration mode (switch is running).");
        } else {
            if !self.start(net, con, now) {
                return false;
            }
            self.persist = false;
            con.println("Entering switch configuration mode.");
            con.println("Type 'help' for commands, 'exit' to leave. 'on' keeps it running after exit.");
        }
        self.session = Some(CliSession::new());
        true
    }

    /// `switch on`
    pub fn start_detached(&mut self, net: &mut Net, con: &mut dyn Term, now: i64) {
        if Self::running(net) {
            self.persist = true;
            con.println("Switch is already running.");
        } else if self.start(net, con, now) {
            self.persist = true;
            con.println("Switch started. Forwarding in background. Use 'switch off' to stop.");
        }
    }

    /// `switch off`
    pub fn stop_cmd(&mut self, net: &mut Net, con: &mut dyn Term) {
        if Self::running(net) {
            self.stop(net);
            con.println("Switch stopped.");
        } else {
            con.println("Switch is not running.");
        }
    }

    /// Leave the CLI (exit at top level, `end`, or Ctrl+T).
    pub fn leave_cli(&mut self, net: &mut Net, con: &mut dyn Term) {
        self.session = None;
        if self.persist {
            con.println("Exited switch mode. Switch continues running in background.");
            con.println("Use 'switch off' to stop it.");
        } else {
            net.detach_bridge();
            con.println("Exited switch mode.");
        }
    }

    /// Run one line of the local CLI.
    pub fn exec(&mut self, line: &str, net: &mut Net, con: &mut dyn Term, now: i64) -> CliState {
        let Some(mut session) = self.session.take() else { return CliState::Closed };
        let exit = self.exec_in(&mut session, line, net, con, now);
        if exit {
            self.leave_cli(net, con);
            CliState::Closed
        } else if Self::running(net) {
            self.session = Some(session);
            CliState::Open
        } else {
            CliState::Closed
        }
    }

    /// Run one CLI line in `session` (the local CLI or a remote SSH one).
    /// Returns true if the line asked to leave the CLI.
    pub fn exec_in(&mut self, session: &mut CliSession, line: &str, net: &mut Net, con: &mut dyn Term, now: i64) -> bool {
        let Some(bridge) = net.bridge_mut() else { return true };
        let r = cli::exec(bridge, session, line, now);
        if !r.output.is_empty() {
            con.print(&r.output);
            if !r.output.ends_with('\n') {
                con.println("");
            }
        }
        let mut exit = false;
        for e in r.effects {
            match e {
                CliEffect::SviAddress { .. } | CliEffect::SviRemove { .. } => Self::apply_config_effect(net, e, now),
                CliEffect::SaveConfig(text) => {
                    if fs::write(CONFIG_PATH, text.as_bytes()) {
                        con.println(&format!("Configuration saved to /{}.", CONFIG_PATH));
                    } else {
                        con.println("% Failed to write configuration.");
                    }
                }
                CliEffect::Detach => self.persist = true,
                CliEffect::ExitCli => exit = true,
            }
        }
        net.sync_bridge_ports();
        exit
    }

    /// Remote (SSH) CLI entry: start the switch detached if needed, so
    /// closing the SSH session never stops it.
    pub fn enter_remote(&mut self, net: &mut Net, con: &mut dyn Term, now: i64) -> Option<CliSession> {
        if !Self::running(net) {
            self.start_detached(net, con, now);
            if !Self::running(net) {
                return None;
            }
        } else {
            con.println("Entering switch configuration mode (switch is running).");
        }
        Some(CliSession::new())
    }
}
