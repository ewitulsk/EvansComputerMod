//! The requirement checks and their messages, and `connect` end to end against
//! the fake kernel and the ecm-wifi test AP (shared with wpa_supplicant's tests).

use super::*;

#[path = "../../../crates/ecm-wifi/tests/common/ap.rs"]
mod test_ap;
#[path = "../../wpa_supplicant/src/fake_kernel.rs"]
mod fake_kernel;
use fake_kernel::{FakeKernel, STA};

const PASS: &str = "correct horse battery";

struct TestHost {
    k: FakeKernel,
    modules: Vec<String>,
}

impl Port for TestHost {
    fn ctl(&mut self, req: &str) -> Option<(i32, Vec<u8>)> {
        self.k.ctl(req)
    }
    fn eapol_send(&mut self, eth: &[u8]) -> bool {
        self.k.eapol_send(eth)
    }
    fn eapol_recv(&mut self) -> Option<Vec<u8>> {
        self.k.eapol_recv()
    }
    fn now_ms(&mut self) -> u64 {
        self.k.now_ms()
    }
    fn log(&mut self, line: &str) {
        self.k.log(line)
    }
}

impl Host for TestHost {
    fn wifi_module_modes(&mut self) -> Vec<String> {
        self.modules.clone()
    }
    fn sleep_ms(&mut self, ms: u64) {
        for _ in 0..ms {
            self.k.tick();
        }
    }
}

fn host(pass: Option<&str>) -> TestHost {
    TestHost { k: FakeKernel::new(pass), modules: vec!["wifi".into()] }
}

fn rand() -> Box<dyn FnMut(&mut [u8])> {
    let mut c = 7u8;
    Box::new(move |b: &mut [u8]| b.iter_mut().for_each(|x| {
        c = c.wrapping_mul(31).wrapping_add(17);
        *x = c
    }))
}

fn entry(ssid: &str, signal: i32, security: &str) -> ScanEntry {
    ScanEntry { bssid: "02:aa:00:00:00:01".into(), freq: 2437, signal_dbm: signal, security: security.into(), channel: 6, ssid: ssid.as_bytes().to_vec() }
}

#[test]
fn missing_module_controller_mode_and_monitor_mode_each_say_how_to_fix() {
    let none = check_interface(Some("present=0\n"), &[]).unwrap_err();
    assert!(none.what.contains("no Wi-Fi Module"), "{:?}", none);
    assert!(none.render().contains("Module Expansion Card") && none.render().contains("Wi-Fi Module and right-click"));

    let ctl = check_interface(Some("present=0\n"), &["controller".into()]).unwrap_err();
    assert!(ctl.what.contains("controller mode") && ctl.fix[0] == "Run: wifi mode wifi", "{:?}", ctl);

    let mon = check_interface(Some("present=1\nmac=02:ec:00:00:00:42\nmode=monitor\n"), &[]).unwrap_err();
    assert!(mon.fix[0].contains("iw dev wlan0 set type managed"));

    assert!(check_interface(None, &[]).unwrap_err().what.contains("no Wi-Fi support"));
    assert_eq!(check_interface(Some("present=1\nmac=02:ec:00:00:00:42\nmode=managed\n"), &[]).unwrap(), STA);
}

#[test]
fn network_choice_checks_range_name_and_password() {
    let empty = choose(&[], "ecm-lab", Some(PASS)).unwrap_err();
    assert!(empty.what.contains("no Wi-Fi networks are in range") && empty.render().contains("stone, water and iron"));

    let heard = [entry("ECM-Lab", -50, "wpa2-psk"), entry("cafe", -70, "open")];
    let wrong = choose(&heard, "ecm-lab", Some(PASS)).unwrap_err();
    assert!(wrong.what.contains("'ECM-Lab' (-50 dBm)") && wrong.what.contains("'cafe' (-70 dBm)"), "{:?}", wrong);
    assert!(wrong.fix[0].contains("did you mean 'ECM-Lab'"));

    let lab = [entry("ecm-lab", -90, "wpa2-psk"), entry("ecm-lab", -55, "wpa2-psk")];
    let need = choose(&lab, "ecm-lab", None).unwrap_err();
    assert!(need.what.contains("needs a password") && need.fix[0] == "Run: wifi connect ecm-lab <password>");
    assert!(choose(&lab, "ecm-lab", Some("short")).unwrap_err().what.contains("8 to 63 characters"));

    let ok = choose(&lab, "ecm-lab", Some(PASS)).unwrap();
    assert_eq!((ok.network.signal_dbm, ok.secured, ok.warnings.len()), (-55, true, 0), "strongest access point, no warnings");

    let weak = choose(&[entry("far away", -86, "open")], "far away", Some("whatever")).unwrap();
    assert!(!weak.secured && weak.warnings.len() == 2, "{:?}", weak.warnings);
    assert!(weak.warnings[1].contains("weak (-86 dBm)"));

    assert!(choose(&[entry("old", -50, "unsupported")], "old", None).unwrap_err().what.contains("can't join"));
    assert_eq!(quote("my net"), "\"my net\"");
}

#[test]
fn connects_to_a_wpa2_network_through_scan_and_handshake() {
    let mut h = host(Some(PASS));
    let found = scan(&mut h).unwrap();
    assert!(found.iter().any(|e| e.ssid == b"ecm-lab" && e.security == "wpa2-psk"), "{:?}", found);
    assert!(format_scan(&found).contains("ecm-lab") && format_scan(&found).contains("WPA2 (password)"));
    let choice = choose(&found, "ecm-lab", Some(PASS)).unwrap();
    let joined = join(&mut h, STA, &choice, Some(PASS), rand()).unwrap();
    assert_eq!(joined.channel, 6);
    assert!(h.k.wlan.authorized(), "keys installed: the link carries traffic after the program exits");
}

#[test]
fn a_wrong_password_is_reported_as_one() {
    let mut h = host(Some(PASS));
    let found = scan(&mut h).unwrap();
    let choice = choose(&found, "ecm-lab", Some("not the password")).unwrap();
    let e = join(&mut h, STA, &choice, Some("not the password"), rand()).unwrap_err();
    assert!(e.what.contains("password for 'ecm-lab' is wrong"), "{:?}", e);
    assert!(!h.k.wlan.authorized());
}

#[test]
fn joins_an_open_network() {
    let mut h = host(None);
    let found = scan(&mut h).unwrap();
    let choice = choose(&found, "ecm-lab", None).unwrap();
    assert!(!choice.secured);
    join(&mut h, STA, &choice, None, rand()).unwrap();
}

#[test]
fn saved_networks_replace_the_same_name_and_keep_others() {
    let first = save_network(None, conf_network("ecm-lab", Some("old password"))).unwrap();
    let both = save_network(Some(&first), conf_network("cafe", None)).unwrap();
    let again = save_network(Some(&both), conf_network("ecm-lab", Some(PASS))).unwrap();
    let c = conf::parse(&again).unwrap();
    assert_eq!(c.networks.len(), 2);
    assert!(again.contains(PASS) && !again.contains("old password") && again.contains("key_mgmt=NONE"), "{}", again);
}

/// A config file that doesn't parse (here: a network block missing its
/// closing brace after two good ones) is not overwritten: saving refuses
/// with the parse error, so the other entries aren't silently dropped.
#[test]
fn unparsable_config_is_not_overwritten() {
    let good = save_network(None, conf_network("home", Some("home password"))).unwrap();
    let good = save_network(Some(&good), conf_network("office", Some("office password"))).unwrap();
    let broken = format!("{good}network={{
	ssid=\"half\"
");
    let e = save_network(Some(&broken), conf_network("cafe", None)).unwrap_err();
    assert!(e.contains("line") && e.contains("not closed"), "{e}");
    // An empty file is fine (no networks yet).
    assert!(save_network(Some(""), conf_network("cafe", None)).unwrap().contains("cafe"));
}

#[test]
fn status_text_and_dhcp_advice() {
    let up = format!(
        "present=1\nstate=ASSOCIATED\nauthorized=1\nbssid=02:aa:00:00:00:01\nssid={}\nbss_channel=6\nsignal=-52\ntarget_security=wpa2\n",
        ssid_hex("ecm-lab")
    );
    let s = format_status(&up, Some("192.168.50.10/24"));
    assert!(s.contains("connected to 'ecm-lab'") && s.contains("signal -52 dBm, WPA2") && s.contains("192.168.50.10/24"), "{}", s);
    assert!(format_status("present=1\nstate=SCANNING\n", None).contains("not connected"));
    let d = no_lease("ecm-lab", "SELECTING", 10).render();
    assert!(d.contains("dhcpd &") && d.contains("Internet Gateway doesn't hand out") && d.contains("ifconfig wlan0"), "{}", d);
}
