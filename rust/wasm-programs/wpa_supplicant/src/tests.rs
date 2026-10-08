//! Config / argument parsing, and the daemon's state machine against a fake
//! kernel (ecm-wifi `Wlan` behind the same control-channel requests) and the
//! ecm-wifi test AP.

use std::collections::VecDeque;

use ecm_wifi::frame::{self, MacAddr};
use ecm_wifi::mlme::{Action, Event};
use ecm_wifi::{MlmeConfig, RxOutput, ScanRequest, Wlan};

use super::conf::{self, Psk};
use super::*;

#[path = "../../../crates/ecm-wifi/tests/common/ap.rs"]
mod test_ap;
use test_ap::TestAp;

const STA: MacAddr = [0x02, 0xec, 0, 0, 0, 0x42];
const AP: MacAddr = [0x02, 0xaa, 0, 0, 0, 0x01];
const PASS: &str = "correct horse battery";

fn args(s: &str) -> Result<Args, String> {
    parse_args(&s.split_whitespace().map(String::from).collect::<Vec<_>>())
}

#[test]
fn parses_command_line() {
    let a = args("-B -i wlan0 -c /etc/wpa_supplicant.conf").unwrap();
    assert!(a.background && !a.debug);
    assert_eq!((a.ifname.as_str(), a.conf.as_str()), ("wlan0", "/etc/wpa_supplicant.conf"));
    let a = args("-iwlan0 -dd -Dnl80211 -f /var/log/wpa.log").unwrap();
    assert_eq!(a.ifname, "wlan0");
    assert!(a.debug && !a.background);
    assert_eq!(a.driver, None);
    assert_eq!(a.log_file.as_deref(), Some("/var/log/wpa.log"));
    assert_eq!(a.conf, DEFAULT_CONF);
    assert_eq!(args("-i wlan0 -D packet").unwrap().driver.as_deref(), Some("packet"));
    assert!(args("-B").unwrap_err().contains("no interface"));
    assert!(args("-i").unwrap_err().contains("requires an argument"));
    assert!(args("-i wlan0 -x").is_err());
    assert!(args("-i wlan0 -D bogus").is_err());
}

#[test]
fn parses_network_blocks() {
    let text = r#"
ctrl_interface=/run/wpa_supplicant   # comment
network={
    ssid="ecm-lab"
    psk="correct horse battery"
    priority=5
}
network={
	ssid=636166 # hex "caf"
	key_mgmt=NONE
	bssid=02:aa:00:00:00:02
	disabled=1
}
network={
    ssid="raw#key"
    psk=00112233445566778899aabbccddeeff00112233445566778899AABBCCDDEEFF
}
"#;
    let c = conf::parse(text).unwrap();
    assert_eq!(c.globals, vec!["ctrl_interface=/run/wpa_supplicant"]);
    assert_eq!(c.networks.len(), 3);
    assert_eq!(c.networks[0].ssid, b"ecm-lab");
    assert_eq!(c.networks[0].psk, Some(Psk::Passphrase(PASS.into())));
    assert_eq!(c.networks[0].priority, 5);
    assert_eq!(c.networks[1].ssid, b"caf");
    assert!(c.networks[1].open && c.networks[1].disabled);
    assert_eq!(c.networks[1].bssid, Some([2, 0xaa, 0, 0, 0, 2]));
    assert_eq!(c.networks[2].ssid, b"raw#key", "# inside quotes is not a comment");
    assert!(matches!(&c.networks[2].psk, Some(Psk::Hex(h)) if h.ends_with("aabbccddeeff")));
    // Round trip through the writer.
    assert_eq!(conf::parse(&conf::render(&c)).unwrap(), c);
    // The supplicant view derives the PMK (IEEE 802.11 Annex J vector for the passphrase).
    let n = conf::ConfNetwork { ssid: b"IEEE".to_vec(), psk: Some(Psk::Passphrase("password".into())), ..Default::default() };
    let nc = n.to_network_config().unwrap();
    assert_eq!(&nc.psk.unwrap()[..4], &[0xf4, 0x2c, 0x6f, 0xc5]);
}

#[test]
fn rejects_bad_config_with_line_numbers() {
    let e = conf::parse("network={\n ssid=\"x\"\n}\n").unwrap_err();
    assert!(e.msg.contains("psk") && e.line == 1, "{e}");
    assert_eq!(conf::parse("network={\n psk=\"short\"\n").unwrap_err().line, 2);
    assert!(conf::parse("network={\n ssid=\"x\"\n frob=1\n}").unwrap_err().msg.contains("unknown"));
    assert!(conf::parse("network={\n ssid=\"x\"\n key_mgmt=NONE\n").unwrap_err().msg.contains("not closed"));
    assert!(conf::parse("garbage\n").is_err());
}

/// The kernel's `wlan0` control channel, played by ecm-wifi's `Wlan` and the test AP.
struct FakeKernel {
    now: u64,
    wlan: Wlan,
    ap: TestAp,
    channel: u8,
    events: Vec<(u64, String)>,
    eapol: VecDeque<Vec<u8>>,
    log: Vec<String>,
    requests: Vec<String>,
}

impl FakeKernel {
    fn new(pass: Option<&str>) -> FakeKernel {
        FakeKernel {
            now: 1000,
            wlan: Wlan::new(MlmeConfig::new(STA)),
            ap: TestAp::new(AP, "ecm-lab", 6, pass),
            channel: 1,
            events: Vec::new(),
            eapol: VecDeque::new(),
            log: Vec::new(),
            requests: Vec::new(),
        }
    }

    fn push(&mut self, e: String) {
        let seq = self.events.len() as u64 + 1;
        self.events.push((seq, e));
    }

    fn pump(&mut self) {
        for _ in 0..200 {
            let mut progress = false;
            while let Some(a) = self.wlan.pop_action() {
                progress = true;
                match a {
                    Action::Transmit { frame, .. } => {
                        if self.channel == self.ap.channel {
                            self.ap.on_rx(&frame, self.now)
                        }
                    }
                    Action::SetChannel(c) => self.channel = c,
                    Action::SetRxFilter(_) => {}
                    Action::Event(Event::ScanDone { results, aborted }) => {
                        self.push(format!("SCAN_DONE results={} aborted={}", results, aborted as u8))
                    }
                    Action::Event(Event::Connected { bssid, ssid, channel, ap_rsn_ie, sta_rsn_ie, .. }) => self.push(format!(
                        "CONNECTED bssid={} ssid={} channel={} aid=1 ap_rsn={} sta_rsn={} roamed=0",
                        frame::mac_str(&bssid),
                        hex(&ssid),
                        channel,
                        ap_rsn_ie.map(|x| hex(&x)).unwrap_or("-".into()),
                        sta_rsn_ie.map(|x| hex(&x)).unwrap_or("-".into())
                    )),
                    Action::Event(Event::Disconnected { bssid, reason }) => self.push(format!(
                        "DISCONNECTED bssid={} reason={} kind=x local={}",
                        frame::mac_str(&bssid),
                        reason.code(),
                        reason.locally_generated() as u8
                    )),
                    Action::Event(Event::ConnectFailed { .. }) => self.push("CONNECT_FAILED bssid=- reason=x".into()),
                }
            }
            while let Some(f) = self.ap.out.pop_front() {
                progress = true;
                if self.channel != self.ap.channel {
                    continue;
                }
                if let Some(RxOutput::Ethernet(eth)) = self.wlan.on_rx(&f, -45, 108, self.ap.channel, self.now) {
                    if frame::ethertype(&eth) == Some(frame::ETHERTYPE_EAPOL) {
                        self.eapol.push_back(eth);
                    }
                }
            }
            if !progress {
                return;
            }
        }
    }

    fn tick(&mut self) {
        self.now += 1;
        self.ap.poll(self.now);
        self.wlan.poll(self.now);
        self.pump();
    }
}

impl Port for FakeKernel {
    fn ctl(&mut self, req: &str) -> Option<(i32, Vec<u8>)> {
        self.requests.push(req.split(' ').next().unwrap().to_string());
        let p: Vec<&str> = req.split_whitespace().collect();
        let now = self.now;
        let r = match p.as_slice() {
            ["status"] => (0, format!("present=1\nmac={}\nmode=managed\nevent_seq={}\n", frame::mac_str(&STA), self.events.len()).into_bytes()),
            ["events", after] => {
                let a: u64 = after.parse().unwrap();
                let s: String = self.events.iter().filter(|(q, _)| *q > a).map(|(q, e)| format!("{} {}\n", q, e)).collect();
                (0, s.into_bytes())
            }
            ["scan"] => (if self.wlan.scan(ScanRequest::default(), now) { 0 } else { -16 }, vec![]),
            ["scan_results"] => {
                let s: String = self
                    .wlan
                    .mlme()
                    .scan_results(now)
                    .iter()
                    .map(|b| {
                        let sec = if b.security() == frame::Security::Wpa2Psk { "wpa2-psk" } else { "open" };
                        format!("{} {} {} {} {} {}\n", frame::mac_str(&b.bssid), 2437, b.rssi, sec, b.channel, hex(&b.ssid))
                    })
                    .collect();
                (0, s.into_bytes())
            }
            ["connect", ssid, sec, ..] => {
                let s = ecm_host_abi::wifi::unhex(ssid).unwrap();
                let security = if *sec == "wpa2" { ecm_wifi::ConnectSecurity::Wpa2Psk } else { ecm_wifi::ConnectSecurity::Open };
                self.wlan.connect(ecm_wifi::ConnectParams::new(&s, security), now);
                (0, vec![])
            }
            ["disconnect", ..] => {
                self.wlan.disconnect(3, now);
                (0, vec![])
            }
            ["install_ptk", b, tk] => {
                let mut k = [0u8; 16];
                k.copy_from_slice(&ecm_host_abi::wifi::unhex(tk).unwrap());
                (if self.wlan.install_ptk(parse_mac(b).unwrap(), k) { 0 } else { 1 }, vec![])
            }
            ["install_gtk", id, rsc, g] => {
                let ok = self.wlan.install_gtk(id.parse().unwrap(), &ecm_host_abi::wifi::unhex(g).unwrap(), rsc.parse().unwrap());
                (if ok { 0 } else { 1 }, vec![])
            }
            _ => (-22, vec![]),
        };
        self.pump();
        Some(r)
    }
    fn eapol_send(&mut self, eth: &[u8]) -> bool {
        let Some(tx) = self.wlan.send_ethernet(eth, self.now) else { return false };
        if self.channel == self.ap.channel {
            self.ap.on_rx(&tx.frame, self.now);
        }
        self.pump();
        true
    }
    fn eapol_recv(&mut self) -> Option<Vec<u8>> {
        self.eapol.pop_front()
    }
    fn now_ms(&mut self) -> u64 {
        self.now
    }
    fn log(&mut self, line: &str) {
        self.log.push(line.to_string());
    }
}

fn driver(pass: &str) -> Driver {
    let mut c = 0u8;
    let nets = vec![conf::ConfNetwork { ssid: b"ecm-lab".to_vec(), psk: Some(Psk::Passphrase(pass.into())), ..Default::default() }
        .to_network_config()
        .unwrap()];
    Driver::new(STA, "wlan0", Box::new(move |b: &mut [u8]| b.iter_mut().for_each(|x| { c = c.wrapping_add(13); *x = c })), nets)
}

fn run(d: &mut Driver, k: &mut FakeKernel, ms: u64, mut until: impl FnMut(&Driver, &FakeKernel) -> bool) -> bool {
    for _ in 0..ms {
        if until(d, k) {
            return true;
        }
        d.step(k);
        k.tick();
    }
    until(d, k)
}

#[test]
fn scans_selects_network_and_completes_wpa2_handshake() {
    let mut k = FakeKernel::new(Some(PASS));
    let mut d = driver(PASS);
    d.attach(&mut k).unwrap();
    assert!(run(&mut d, &mut k, 3000, |d, _| d.state == WpaState::Completed), "state {:?}, log {:?}", d.state, k.log);
    assert!(k.wlan.authorized() && k.ap.sta_done());
    assert!(k.requests.iter().any(|r| r == "install_ptk") && k.requests.iter().any(|r| r == "install_gtk"));
    let st = d.status_text();
    assert!(st.contains("wpa_state=COMPLETED") && st.contains("bssid=02:aa:00:00:00:01") && st.contains("ssid=ecm-lab") && st.contains("key_mgmt=WPA2-PSK"), "{st}");
    assert!(k.log.iter().any(|l| l.starts_with("CTRL-EVENT-CONNECTED - Connection to 02:aa:00:00:00:01 completed [id=0]")), "{:?}", k.log);
}

#[test]
fn wrong_passphrase_reports_possible_wrong_key_and_never_completes() {
    let mut k = FakeKernel::new(Some(PASS));
    let mut d = driver("not the right one");
    d.attach(&mut k).unwrap();
    assert!(!run(&mut d, &mut k, 2500, |d, _| d.state == WpaState::Completed));
    assert!(!k.wlan.has_ptk());
    assert!(k.log.iter().any(|l| l.contains("pre-shared key may be incorrect") || l.contains("timed out") || l.contains("CTRL-EVENT-DISCONNECTED")), "{:?}", k.log);
}

#[test]
fn no_matching_network_stays_disconnected_and_rescans() {
    let mut k = FakeKernel::new(Some(PASS));
    k.ap.ssid = b"other".to_vec();
    let mut d = driver(PASS);
    d.attach(&mut k).unwrap();
    run(&mut d, &mut k, 1500, |_, _| false);
    assert_eq!(d.state, WpaState::Disconnected);
    assert!(!k.requests.iter().any(|r| r == "connect"));
    // After RESCAN_MS it scans again.
    run(&mut d, &mut k, RESCAN_MS + 100, |_, _| false);
    assert!(k.requests.iter().filter(|r| *r == "scan").count() >= 2);
}

#[test]
fn user_disconnect_stops_reconnecting_until_reconnect() {
    let mut k = FakeKernel::new(None);
    let nets = vec![conf::ConfNetwork { ssid: b"ecm-lab".to_vec(), open: true, ..Default::default() }.to_network_config().unwrap()];
    let mut d = Driver::new(STA, "wlan0", Box::new(|_: &mut [u8]| {}), nets);
    d.attach(&mut k).unwrap();
    assert!(run(&mut d, &mut k, 2000, |d, _| d.state == WpaState::Completed));
    d.command(&mut k, "disconnect");
    run(&mut d, &mut k, RESCAN_MS, |_, _| false);
    assert_eq!(d.state, WpaState::Disconnected);
    assert!(k.wlan.mlme().current_bss().is_none());
    d.command(&mut k, "reconnect");
    assert!(run(&mut d, &mut k, 2000, |d, _| d.state == WpaState::Completed));
}
