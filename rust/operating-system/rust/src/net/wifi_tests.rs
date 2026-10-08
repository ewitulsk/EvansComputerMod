//! `wlan0` adapter tests: the real `Net` dispatcher with an in-memory SoftMAC
//! radio joined to the ecm-wifi test AP (hostapd stand-in). Userspace is
//! played through the same `WIFI_CTL` text requests the programs send.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use ecm_wifi::frame::{self, MacAddr};
use ecm_wifi::supplicant::{NetworkConfig, Supplicant, SupplicantOutput};

use super::*;
use crate::net::{Net, Nics};

#[path = "../../../../crates/ecm-wifi/tests/common/ap.rs"]
mod test_ap;
use test_ap::TestAp;

const STA: MacAddr = [0x02, 0xec, 0x00, 0x00, 0x00, 0x42];
const AP: MacAddr = [0x02, 0xaa, 0, 0, 0, 0x01];
const HOST: MacAddr = [0x02, 0x00, 0, 0, 0, 0x99];
const PASS: &str = "correct horse battery";

#[derive(Default)]
struct Air {
    present: usize,
    channel: u8,
    filter_mode: i32,
    filter_bssid: Option<MacAddr>,
    to_sta: VecDeque<(Vec<u8>, RxMeta)>,
    from_sta: Vec<(Vec<u8>, u32, i8)>,
    status: VecDeque<TxStatus>,
    /// Ack every unicast frame (the Java low MAC does this on the medium).
    ack: bool,
}

#[derive(Clone)]
struct MockRadio(Rc<RefCell<Air>>);

impl WifiRadio for MockRadio {
    fn present(&self) -> usize {
        self.0.borrow().present
    }
    fn mac(&self) -> Option<MacAddr> {
        Some(STA)
    }
    fn tx(&mut self, f: &[u8], rate_kbps: u32, power: i8) -> bool {
        let mut a = self.0.borrow_mut();
        a.from_sta.push((f.to_vec(), rate_kbps, power));
        let fc = u16::from_le_bytes([f[0], f[1]]);
        let seq = u16::from_le_bytes([f[22], f[23]]);
        let acked = a.ack || frame::is_multicast(&f[4..10].try_into().unwrap());
        a.status.push_back(TxStatus { acked, attempts: if acked { 1 } else { 8 }, rate_kbps, seq_ctrl: seq, frame_control: fc });
        true
    }
    fn rx(&mut self, buf: &mut [u8]) -> Option<(usize, RxMeta)> {
        let (f, m) = self.0.borrow_mut().to_sta.pop_front()?;
        buf[..f.len()].copy_from_slice(&f);
        Some((f.len(), m))
    }
    fn set_channel(&mut self, ch: u8) -> bool {
        self.0.borrow_mut().channel = ch;
        true
    }
    fn set_rx_filter(&mut self, mode: i32, bssid: Option<MacAddr>) -> bool {
        let mut a = self.0.borrow_mut();
        a.filter_mode = mode;
        a.filter_bssid = bssid;
        true
    }
    fn tx_status(&mut self) -> Option<TxStatus> {
        self.0.borrow_mut().status.pop_front()
    }
}

struct NoNics;
impl Nics for NoNics {
    fn count(&self) -> usize {
        0
    }
    fn mac(&self, _: usize) -> Option<[u8; 6]> {
        None
    }
    fn tx(&mut self, _: usize, _: &[u8]) {}
    fn rx(&mut self, _: &mut [u8]) -> Option<(usize, usize)> {
        None
    }
    fn set_promiscuous(&mut self, _: usize, _: bool) {}
    fn carrier(&self, _: usize) -> bool {
        false
    }
}

/// Net + radio + test AP + (optionally) a supplicant driven over WIFI_CTL.
struct Bench {
    now: i64,
    net: Net,
    air: Rc<RefCell<Air>>,
    ap: TestAp,
    supp: Option<Supplicant>,
    event_seq: u64,
    events: Vec<String>,
}

impl Bench {
    fn new(present: bool, passphrase: Option<&str>) -> Bench {
        let air = Rc::new(RefCell::new(Air { present: present as usize, channel: 1, ack: true, ..Default::default() }));
        let net = Net::with_radios(Box::new(NoNics), Box::new(MockRadio(air.clone())), 7, 1000);
        Bench { now: 1000, net, air, ap: TestAp::new(AP, "ecm-lab", 6, passphrase), supp: None, event_seq: 0, events: Vec::new() }
    }

    fn ctl(&mut self, req: &str) -> (i32, String) {
        let (s, r) = self.net.wifi_ctl(req.as_bytes(), self.now);
        (s, String::from_utf8_lossy(&r).into_owned())
    }

    fn passes(&self, f: &[u8]) -> bool {
        let a = self.air.borrow();
        if a.filter_mode == RX_FILTER_MONITOR {
            return true;
        }
        let a1: MacAddr = f[4..10].try_into().unwrap();
        let to_me = a1 == STA || frame::is_multicast(&a1);
        match a.filter_bssid {
            Some(b) if frame::is_multicast(&a1) => to_me && f[10..16] == b,
            _ => to_me,
        }
    }

    /// One millisecond of air time.
    fn step(&mut self) {
        self.now += 1;
        let now = self.now as u64;
        self.ap.poll(now);
        let on_ch = self.air.borrow().channel == self.ap.channel;
        while let Some(f) = self.ap.out.pop_front() {
            if on_ch && self.passes(&f) {
                let meta = RxMeta { rssi_dbm_x10: self.ap.rssi * 10, rate_kbps: 54000, channel: self.ap.channel, timestamp_us: self.now * 1000, fcs_ok: true };
                self.air.borrow_mut().to_sta.push_back((f, meta));
            }
        }
        self.net.rx_wifi(self.now);
        self.net.poll(self.now);
        let sent: Vec<_> = self.air.borrow_mut().from_sta.drain(..).collect();
        let on_ch = self.air.borrow().channel == self.ap.channel;
        for (f, _, _) in sent {
            if on_ch {
                self.ap.on_rx(&f, now);
            }
        }
        self.drive_supplicant();
    }

    /// What `wpa_supplicant` does each loop, through WIFI_CTL only.
    fn drive_supplicant(&mut self) {
        let (_, ev) = self.ctl(&format!("events {}", self.event_seq));
        for line in ev.lines() {
            let mut it = line.splitn(2, ' ');
            let seq: u64 = it.next().unwrap().parse().unwrap();
            let body = it.next().unwrap_or("").to_string();
            self.event_seq = seq;
            self.events.push(body.clone());
            let Some(supp) = self.supp.as_mut() else { continue };
            if body.starts_with("CONNECTED") {
                let kv = |k: &str| body.split_whitespace().find_map(|t| t.strip_prefix(&format!("{}=", k))).unwrap().to_string();
                let bssid = parse_mac(&kv("bssid")).unwrap();
                let ssid = unhex(&kv("ssid")).unwrap();
                let ap_rsn = unhex(&kv("ap_rsn"));
                let sta_rsn = unhex(&kv("sta_rsn"));
                supp.on_associated(bssid, &ssid, ap_rsn.as_deref(), sta_rsn.as_deref(), self.now as u64);
            } else if body.starts_with("DISCONNECTED") {
                supp.on_disconnected();
            }
        }
        if self.supp.is_none() {
            return;
        }
        loop {
            let (s, f) = self.net.wifi_ctl(b"eapol_rx", self.now);
            if s != 1 {
                break;
            }
            self.supp.as_mut().unwrap().on_ethernet(&f, self.now as u64);
        }
        self.supp.as_mut().unwrap().poll(self.now as u64);
        while let Some(o) = self.supp.as_mut().unwrap().pop_output() {
            match o {
                SupplicantOutput::SendEapol { dst, frame: e } => {
                    let eth = frame::ethernet(&dst, &STA, frame::ETHERTYPE_EAPOL, &e);
                    assert_eq!(self.ctl(&format!("eapol_tx {}", hex(&eth))).0, 0);
                }
                SupplicantOutput::InstallPtk { bssid, tk } => {
                    self.ctl(&format!("install_ptk {} {}", frame::mac_str(&bssid), hex(&tk)));
                }
                SupplicantOutput::InstallGtk { key_id, gtk, rsc } => {
                    self.ctl(&format!("install_gtk {} {} {}", key_id, rsc, hex(&gtk)));
                }
                SupplicantOutput::Disconnect { reason } => {
                    self.ctl(&format!("disconnect {}", reason));
                }
                SupplicantOutput::Event(_) => {}
            }
        }
    }

    fn run_until(&mut self, max_ms: i64, mut cond: impl FnMut(&mut Bench) -> bool) {
        let start = self.now;
        while !cond(self) {
            assert!(self.now - start < max_ms, "timed out; events: {:?}", self.events);
            self.step();
        }
    }

    fn wlan_idx(&self) -> Option<usize> {
        self.net.stack.find_iface("wlan0")
    }
}

fn arp_request(src_mac: &MacAddr, src_ip: [u8; 4], target_ip: [u8; 4]) -> Vec<u8> {
    let mut p = vec![0, 1, 8, 0, 6, 4, 0, 1];
    p.extend_from_slice(src_mac);
    p.extend_from_slice(&src_ip);
    p.extend_from_slice(&[0; 6]);
    p.extend_from_slice(&target_ip);
    frame::ethernet(&frame::BROADCAST, src_mac, 0x0806, &p)
}

#[test]
fn no_module_means_no_wlan0_and_ctl_reports_no_device() {
    let mut b = Bench::new(false, None);
    assert!(b.wlan_idx().is_none());
    assert_eq!(b.ctl("status"), (0, "present=0\n".to_string()));
    assert_eq!(b.ctl("scan").0, ENODEV);
}

#[test]
fn wlan0_appears_when_module_is_hot_plugged() {
    let mut b = Bench::new(false, None);
    b.air.borrow_mut().present = 1;
    b.run_until(2 * PRESENT_POLL_MS, |b| b.wlan_idx().is_some());
    let i = b.wlan_idx().unwrap();
    let f = b.net.stack.iface(i).unwrap();
    assert_eq!(f.mac.0, STA);
    assert!(!f.link_up, "no carrier until associated");
    let (s, st) = b.ctl("status");
    assert_eq!(s, 0);
    assert!(st.contains("mac=02:ec:00:00:00:42") && st.contains("mode=managed"), "{st}");
}

#[test]
fn scan_reports_ap_with_frequency_signal_and_security() {
    let mut b = Bench::new(true, Some(PASS));
    b.ap.rssi = -48;
    assert_eq!(b.ctl("scan").0, 0);
    assert_eq!(b.ctl("scan").0, EBUSY, "second scan while one runs");
    b.run_until(1000, |b| b.events.iter().any(|e| e.starts_with("SCAN_DONE")));
    let (_, res) = b.ctl("scan_results");
    let line = res.lines().find(|l| l.starts_with("02:aa:00:00:00:01")).expect(&res);
    let t: Vec<&str> = line.split(' ').collect();
    assert_eq!(t, vec!["02:aa:00:00:00:01", "2437", "-48", "wpa2-psk", "6", &hex(b"ecm-lab")]);
    // The radio was retuned across the band and probes went out on channel 6.
    assert!(b.events.iter().any(|e| e == "SCAN_DONE results=1 aborted=0"), "{:?}", b.events);
    assert!(b.ap.probe_reqs >= 1);
}

#[test]
fn wpa2_handshake_over_ctl_then_encrypted_arp_both_ways() {
    let mut b = Bench::new(true, Some(PASS));
    let mut ctr = 0u8;
    let mut supp = Supplicant::new(STA, Box::new(move |x: &mut [u8]| {
        for v in x.iter_mut() {
            ctr = ctr.wrapping_add(29);
            *v = ctr;
        }
    }));
    supp.add_network(NetworkConfig::wpa2_passphrase(b"ecm-lab", PASS).unwrap());
    b.supp = Some(supp);
    assert_eq!(b.ctl("eapol_subscribe").0, 0);
    assert_eq!(b.ctl(&format!("connect {} wpa2", hex(b"ecm-lab"))).0, 0);
    b.run_until(3000, |b| b.net.wifi.wlan().is_some_and(|w| w.authorized()) && b.ap.sta_done());
    let i = b.wlan_idx().unwrap();
    assert!(b.net.wifi_carrier() && b.net.stack.iface(i).unwrap().link_up);
    let (_, st) = b.ctl("status");
    assert!(st.contains("authorized=1") && st.contains("ptk=1") && st.contains("bssid=02:aa:00:00:00:01"), "{st}");
    // Bring up an address and answer an ARP request from the wired side (group-keyed
    // broadcast down, pairwise-encrypted reply up through the AP).
    b.net.stack.configure_addr(i, ecm_net::types::Ipv4Addr([10, 0, 0, 2]), 24, b.now);
    b.ap.downlink(&arp_request(&HOST, [10, 0, 0, 1], [10, 0, 0, 2]), false);
    let is_reply = |e: &Vec<u8>| frame::ethertype(e) == Some(0x0806) && e.len() >= 42 && e[20..22] == [0, 2];
    b.run_until(500, |b| b.ap.wired_rx.iter().any(is_reply));
    let reply = b.ap.wired_rx.iter().find(|e| is_reply(e)).unwrap().clone();
    assert_eq!(&reply[0..6], &HOST);
    assert_eq!(&reply[6..12], &STA);
    assert_eq!(&reply[28..32], &[10, 0, 0, 2], "ARP sender IP");
    // Every data frame on the air after the handshake was CCMP-protected.
    let w = b.net.wifi.wlan().unwrap();
    assert!(w.stats().tx_packets >= 3 && w.stats().rx_packets >= 1);
    // link shows the AP and the signal.
    let (_, link) = b.ctl("link");
    assert!(link.contains("Connected to 02:aa:00:00:00:01") && link.contains("SSID: ecm-lab") && link.contains("signal: -50 dBm"), "{link}");
}

#[test]
fn open_network_connects_without_supplicant_and_raises_carrier() {
    let mut b = Bench::new(true, None);
    assert_eq!(b.ctl(&format!("connect {} open", hex(b"ecm-lab"))).0, 0);
    b.run_until(2000, |b| b.net.wifi_carrier());
    assert!(b.events.iter().any(|e| e.starts_with("CONNECTED bssid=02:aa:00:00:00:01")));
    assert_eq!(b.ctl("disconnect").0, 0);
    b.step();
    assert!(!b.net.wifi_carrier());
    assert!(b.events.iter().any(|e| e.starts_with("DISCONNECTED") && e.contains("local=1")), "{:?}", b.events);
}

#[test]
fn unacked_data_frames_reach_rate_control_as_failures() {
    let mut b = Bench::new(true, None);
    b.ctl(&format!("connect {} open", hex(b"ecm-lab")));
    b.run_until(2000, |b| b.net.wifi_carrier());
    b.air.borrow_mut().ack = false;
    let i = b.wlan_idx().unwrap();
    b.net.stack.configure_addr(i, ecm_net::types::Ipv4Addr([10, 0, 0, 2]), 24, b.now);
    let eth = frame::ethernet(&HOST, &STA, 0x0800, &[0x45; 40]);
    assert!(b.net.wifi.send_ethernet(&eth, b.now));
    b.net.rx_wifi(b.now);
    let s = b.net.wifi.wlan().unwrap().stats().clone();
    assert!(s.tx_failed >= 1 && s.tx_retries >= 7, "{s:?}");
}

#[test]
fn monitor_mode_programs_radio_and_queues_radiotap_frames() {
    let mut b = Bench::new(true, None);
    assert_eq!(b.ctl("set_type monitor").0, 0);
    assert_eq!(b.air.borrow().filter_mode, RX_FILTER_MONITOR);
    assert_eq!(b.ctl("set_channel 6").0, 0);
    assert_eq!(b.air.borrow().channel, 6);
    assert_eq!(b.ctl("scan").0, EBUSY, "no scanning in monitor mode");
    assert!(b.ctl("status").1.contains("mode=monitor"));
    b.run_until(500, |b| b.ctl("stats").1.lines().any(|l| l.starts_with("monitor_queued=") && l != "monitor_queued=0"));
    let (s, rt) = b.net.wifi_ctl(b"mon_read", b.now);
    assert_eq!(s, 1);
    let (info, hlen) = ecm_wifi::radiotap::parse(&rt).unwrap();
    assert_eq!(info.rate, Some(108), "54 Mb/s in 500 kb/s units");
    assert_eq!(frame::frame_subtype(&rt[hlen..]), frame::ST_BEACON);
    assert!(!b.net.wifi_carrier());
    assert_eq!(b.ctl("set_type managed").0, 0);
    assert_eq!(b.air.borrow().filter_mode, RX_FILTER_NORMAL);
    assert_eq!(b.net.wifi_ctl(b"mon_read", b.now).0, 0, "monitor queue flushed");
}

#[test]
fn ctl_rejects_malformed_requests() {
    let mut b = Bench::new(true, None);
    assert_eq!(b.ctl("connect zz wpa2").0, EINVAL);
    assert_eq!(b.ctl(&format!("connect {} wep", hex(b"x"))).0, EINVAL);
    assert_eq!(b.ctl("set_channel 200").0, EINVAL);
    assert_eq!(b.ctl("install_ptk 02:aa:00:00:00:01 0011").0, EINVAL);
    assert_eq!(b.ctl("eapol_tx 00").0, EINVAL);
    assert_eq!(b.ctl("frobnicate").0, EINVAL);
    assert_eq!(b.ctl("").0, EINVAL);
}

#[test]
fn rate_codes_round_trip_through_kbps() {
    for r in ecm_wifi::rate::RATES {
        assert_eq!(rate_code_for_kbps(r.kbps), Some(r.code));
        assert_eq!(kbps_for_rate_code(r.code), Some(r.kbps));
    }
    assert_eq!(rate_code_for_kbps(7), None);
}
