//! An ideal in-memory radio medium joining one station (Wlan + Supplicant, wired
//! together the way the kernel and a `wpa_supplicant` program will be) with test APs.

#![allow(dead_code)]

use ecm_wifi::frame::{self, MacAddr};
use ecm_wifi::mlme::{Action, Event, LinkState, MlmeConfig, RxFilter};
use ecm_wifi::supplicant::{NetworkConfig, Supplicant, SupplicantEvent, SupplicantOutput};
use ecm_wifi::{RxOutput, Wlan};

use super::ap::TestAp;

pub const STA_MAC: MacAddr = [0x02, 0x11, 0x22, 0x33, 0x44, 0x55];

pub struct Sim {
    pub now: u64,
    pub sta: Wlan,
    pub supp: Supplicant,
    pub aps: Vec<TestAp>,
    pub sta_channel: u8,
    pub filter: RxFilter,
    pub events: Vec<Event>,
    pub supp_events: Vec<SupplicantEvent>,
    pub installs: Vec<SupplicantOutput>,
    /// Non-EAPOL Ethernet frames delivered up the station's stack.
    pub sta_rx: Vec<Vec<u8>>,
    /// Every frame the APs put on the air (for replay / sniffing tests).
    pub air_from_ap: Vec<Vec<u8>>,
    /// Every frame the station put on the air, with its rate code.
    pub air_from_sta: Vec<(Vec<u8>, u8)>,
    pub monitor_rx: Vec<Vec<u8>>,
    /// Feed scan results to the supplicant's network selection automatically.
    pub auto_connect: bool,
}

impl Sim {
    pub fn new(aps: Vec<TestAp>) -> Sim {
        let mut ctr: u8 = 0;
        let rand = Box::new(move |b: &mut [u8]| {
            for x in b.iter_mut() {
                ctr = ctr.wrapping_add(37);
                *x = ctr;
            }
        });
        Sim {
            now: 1000,
            sta: Wlan::new(MlmeConfig::new(STA_MAC)),
            supp: Supplicant::new(STA_MAC, rand),
            aps,
            sta_channel: 1,
            filter: RxFilter::Idle,
            events: Vec::new(),
            supp_events: Vec::new(),
            installs: Vec::new(),
            sta_rx: Vec::new(),
            air_from_ap: Vec::new(),
            air_from_sta: Vec::new(),
            monitor_rx: Vec::new(),
            auto_connect: false,
        }
    }

    pub fn add_network(&mut self, n: NetworkConfig) {
        self.supp.add_network(n);
    }

    fn passes_filter(&self, f: &[u8]) -> bool {
        let Some((h, _)) = frame::Header::parse(f) else { return self.filter == RxFilter::Monitor };
        let to_me = h.addr1 == STA_MAC || frame::is_multicast(&h.addr1);
        match self.filter {
            RxFilter::Monitor => true,
            RxFilter::Idle => to_me,
            RxFilter::Scan => to_me,
            RxFilter::Bssid(b) => to_me && (h.addr2 == b || h.addr3 == b),
        }
    }

    fn deliver_to_aps(&mut self, f: &[u8]) {
        let now = self.now;
        let ch = self.sta_channel;
        for ap in self.aps.iter_mut().filter(|a| a.channel == ch) {
            ap.on_rx(f, now);
        }
    }

    /// Process queued work until quiescent.
    pub fn pump(&mut self) {
        for _ in 0..1000 {
            let mut progress = false;
            while let Some(a) = self.sta.pop_action() {
                progress = true;
                match a {
                    Action::Transmit { frame, rate, .. } => {
                        self.air_from_sta.push((frame.clone(), rate));
                        self.deliver_to_aps(&frame);
                    }
                    Action::SetChannel(c) => self.sta_channel = c,
                    Action::SetRxFilter(f) => self.filter = f,
                    Action::Event(e) => {
                        self.supp.on_event(&e, self.now);
                        if self.auto_connect {
                            if let Event::ScanDone { .. } = e {
                                if *self.sta.mlme().state() == LinkState::Idle {
                                    let res = self.sta.mlme().scan_results(self.now);
                                    if let Some(p) = self.supp.select_network(&res) {
                                        self.sta.connect(p, self.now);
                                    }
                                }
                            }
                        }
                        self.events.push(e);
                    }
                }
            }
            while let Some(o) = self.supp.pop_output() {
                progress = true;
                match o {
                    SupplicantOutput::SendEapol { dst, frame: eapol } => {
                        let eth = frame::ethernet(&dst, &STA_MAC, frame::ETHERTYPE_EAPOL, &eapol);
                        if let Some(tx) = self.sta.send_ethernet(&eth, self.now) {
                            self.air_from_sta.push((tx.frame.clone(), tx.rate));
                            self.deliver_to_aps(&tx.frame);
                        }
                    }
                    SupplicantOutput::InstallPtk { bssid, tk } => {
                        self.sta.install_ptk(bssid, tk);
                        self.installs.push(SupplicantOutput::InstallPtk { bssid, tk });
                    }
                    SupplicantOutput::InstallGtk { key_id, gtk, rsc } => {
                        self.sta.install_gtk(key_id, &gtk, rsc);
                        self.installs.push(SupplicantOutput::InstallGtk { key_id, gtk, rsc });
                    }
                    SupplicantOutput::Disconnect { reason } => self.sta.disconnect(reason, self.now),
                    SupplicantOutput::Event(e) => self.supp_events.push(e),
                }
            }
            for i in 0..self.aps.len() {
                while let Some(f) = self.aps[i].out.pop_front() {
                    progress = true;
                    self.air_from_ap.push(f.clone());
                    let (ch, rssi) = (self.aps[i].channel, self.aps[i].rssi);
                    if ch != self.sta_channel || !self.passes_filter(&f) {
                        continue;
                    }
                    self.rx_at_sta(&f, rssi, ch);
                }
            }
            if !progress {
                return;
            }
        }
        panic!("pump did not settle");
    }

    /// Deliver one frame to the station as if received over the air.
    pub fn rx_at_sta(&mut self, f: &[u8], rssi: i32, ch: u8) {
        match self.sta.on_rx(f, rssi, 108, ch, self.now) {
            Some(RxOutput::Ethernet(eth)) => {
                if frame::ethertype(&eth) == Some(frame::ETHERTYPE_EAPOL) {
                    self.supp.on_ethernet(&eth, self.now);
                } else {
                    self.sta_rx.push(eth);
                }
            }
            Some(RxOutput::Monitor(m)) => self.monitor_rx.push(m),
            None => {}
        }
    }

    /// Advance time by `ms`, 1 ms at a time.
    pub fn run(&mut self, ms: u64) {
        for _ in 0..ms {
            self.now += 1;
            let now = self.now;
            for ap in self.aps.iter_mut() {
                ap.poll(now);
            }
            self.sta.poll(now);
            self.supp.poll(now);
            self.pump();
        }
    }

    /// Run until `cond` holds (or panic after `max_ms`).
    pub fn run_until(&mut self, max_ms: u64, mut cond: impl FnMut(&Sim) -> bool) -> u64 {
        let start = self.now;
        while !cond(self) {
            if self.now - start > max_ms {
                panic!("condition not met within {max_ms} ms; events: {:?}; supp: {:?}", self.events, self.supp_events);
            }
            self.run(1);
        }
        self.now - start
    }

    pub fn connected_to(&self) -> Option<MacAddr> {
        self.sta.mlme().current_bss().map(|b| b.bssid)
    }

    pub fn ptk_installs(&self) -> usize {
        self.installs.iter().filter(|o| matches!(o, SupplicantOutput::InstallPtk { .. })).count()
    }

    /// Ask the station to scan and connect through network selection, like
    /// `wpa_supplicant` does at start-up.
    pub fn start_wpa_supplicant(&mut self) {
        self.auto_connect = true;
        self.sta.scan(Default::default(), self.now);
        self.pump();
    }
}

/// A minimal IPv4/UDP-ish Ethernet frame for data tests.
pub fn ip_frame(dst: &MacAddr, src: &MacAddr, payload: &[u8]) -> Vec<u8> {
    let mut p = vec![0x45, 0, 0, 0, 0, 0, 0, 0, 64, 17, 0, 0, 10, 0, 0, 2, 10, 0, 0, 1];
    p.extend_from_slice(payload);
    frame::ethernet(dst, src, 0x0800, &p)
}
