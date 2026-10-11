//! A fake kernel `wlan0` control channel for host tests: ecm-wifi's `Wlan`
//! station and the ecm-wifi test AP behind the same requests the real kernel
//! answers. Shared by the wpa_supplicant tests and the `wifi` program's tests
//! (each includes it with `#[path]` next to a `test_ap` module).

use std::collections::VecDeque;

use ecm_host_abi::wifi::hex;
use ecm_wifi::frame::{self, MacAddr};
use ecm_wifi::mlme::{Action, Event};
use ecm_wifi::{MlmeConfig, RxOutput, ScanRequest, Wlan};
use wpa_supplicant::{parse_mac, Port};

use super::test_ap::TestAp;

pub const STA: MacAddr = [0x02, 0xec, 0, 0, 0, 0x42];
pub const AP: MacAddr = [0x02, 0xaa, 0, 0, 0, 0x01];

/// The kernel's `wlan0` control channel, played by ecm-wifi's `Wlan` and the test AP.
pub struct FakeKernel {
    pub now: u64,
    pub wlan: Wlan,
    pub ap: TestAp,
    pub channel: u8,
    pub events: Vec<(u64, String)>,
    pub eapol: VecDeque<Vec<u8>>,
    pub log: Vec<String>,
    pub requests: Vec<String>,
}

impl FakeKernel {
    pub fn new(pass: Option<&str>) -> FakeKernel {
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

    pub fn push(&mut self, e: String) {
        let seq = self.events.len() as u64 + 1;
        self.events.push((seq, e));
    }

    pub fn pump(&mut self) {
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

    pub fn tick(&mut self) {
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
