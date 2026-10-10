//! Config / argument parsing, and the daemon's state machine against a fake
//! kernel (ecm-wifi `Wlan` behind the same control-channel requests) and the
//! ecm-wifi test AP.

use super::conf::{self, Psk};
use super::*;

#[path = "../../../crates/ecm-wifi/tests/common/ap.rs"]
mod test_ap;

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

#[path = "fake_kernel.rs"]
mod fake_kernel;
use fake_kernel::{FakeKernel, STA};


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
    run(&mut d, &mut k, 2500, |_, _| false); // one full scan (13 channels x the active dwell)
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
