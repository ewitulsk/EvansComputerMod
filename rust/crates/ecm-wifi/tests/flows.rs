//! End-to-end flows: the station (Wlan + Supplicant) against in-test APs over an ideal
//! medium. Scan, auth, assoc, 4-way handshake, encrypted data both ways, negative
//! cases (wrong passphrase, replay, tampering, downgrade), deauth, beacon loss,
//! retransmissions, group rekey, roaming and monitor mode.

mod common;

use common::ap::TestAp;
use common::sim::{ip_frame, Sim, STA_MAC};
use ecm_wifi::frame::{self, Security};
use ecm_wifi::mlme::{ConnectFailure, ConnectParams, ConnectSecurity, DisconnectReason, Event, ScanRequest};
use ecm_wifi::radiotap;
use ecm_wifi::supplicant::{HandshakeState, NetworkConfig, SupplicantEvent, SupplicantOutput};

const AP1: [u8; 6] = [0x02, 0xaa, 0, 0, 0, 0x01];
const AP2: [u8; 6] = [0x02, 0xaa, 0, 0, 0, 0x02];
const HOST: [u8; 6] = [0x02, 0x00, 0, 0, 0, 0x99];
const PASS: &str = "correct horse battery";

fn wpa2_sim() -> Sim {
    let mut sim = Sim::new(vec![TestAp::new(AP1, "ecm-lab", 6, Some(PASS))]);
    sim.add_network(NetworkConfig::wpa2_passphrase(b"ecm-lab", PASS).unwrap());
    sim
}

fn connect_and_authorize(sim: &mut Sim) {
    sim.start_wpa_supplicant();
    sim.run_until(3000, |s| s.sta.authorized() && s.aps[0].sta_done());
}

#[test]
fn scan_finds_ap_with_ssid_channel_rssi_security() {
    let mut aps = vec![TestAp::new(AP1, "ecm-lab", 6, Some(PASS)), TestAp::new(AP2, "cafe", 11, None)];
    aps[0].rssi = -42;
    aps[1].rssi = -67;
    let mut sim = Sim::new(aps);
    assert!(sim.sta.scan(ScanRequest::default(), sim.now));
    sim.pump();
    let t = sim.run_until(2000, |s| s.events.iter().any(|e| matches!(e, Event::ScanDone { .. })));
    // 13 channels × 30 ms active dwell.
    assert!(t >= 13 * 30 && t <= 13 * 30 + 20, "scan took {t} ms");
    let res = sim.sta.mlme().scan_results(sim.now);
    assert_eq!(res.len(), 2);
    let a = res.iter().find(|b| b.bssid == AP1).unwrap();
    assert_eq!(a.ssid, b"ecm-lab");
    assert_eq!(a.channel, 6);
    assert_eq!(a.rssi, -42);
    assert_eq!(a.security(), Security::Wpa2Psk);
    assert!(a.ht_mcs == 0xff);
    let c = res.iter().find(|b| b.bssid == AP2).unwrap();
    assert_eq!((c.channel, c.rssi, c.security()), (11, -67, Security::Open));
    assert_eq!(res[0].bssid, AP1, "strongest first");
    // Probe requests were sent on every channel.
    let probes = sim.air_from_sta.iter().filter(|(f, _)| frame::frame_subtype(f) == frame::ST_PROBE_REQ).count();
    assert_eq!(probes, 13);
    assert!(sim.aps[0].probe_reqs >= 1);
}

#[test]
fn passive_scan_collects_beacons_only() {
    let mut sim = Sim::new(vec![TestAp::new(AP1, "ecm-lab", 1, Some(PASS))]);
    sim.sta.scan(ScanRequest { ssid: None, channels: vec![1, 6], passive: true }, sim.now);
    sim.pump();
    let t = sim.run_until(1000, |s| s.events.iter().any(|e| matches!(e, Event::ScanDone { .. })));
    assert!(t >= 220, "two passive dwells of 110 ms, took {t}");
    assert!(sim.air_from_sta.is_empty(), "passive scan transmits nothing");
    assert_eq!(sim.sta.mlme().scan_results(sim.now).len(), 1);
}

#[test]
fn hidden_ssid_found_only_by_directed_probe() {
    let mut ap = TestAp::new(AP1, "secret", 3, Some(PASS));
    ap.hidden = true;
    let mut sim = Sim::new(vec![ap]);
    sim.sta.scan(ScanRequest { ssid: None, channels: vec![3], passive: true }, sim.now);
    sim.pump();
    sim.run_until(1000, |s| s.events.iter().any(|e| matches!(e, Event::ScanDone { .. })));
    let r = sim.sta.mlme().scan_results(sim.now);
    assert_eq!(r.len(), 1, "beacon still seen");
    assert!(r[0].hidden());
    sim.events.clear();
    sim.sta.scan(ScanRequest { ssid: Some(b"secret".to_vec()), ..Default::default() }, sim.now);
    sim.pump();
    sim.run_until(1000, |s| s.events.iter().any(|e| matches!(e, Event::ScanDone { .. })));
    let r = sim.sta.mlme().scan_results(sim.now);
    assert_eq!(r[0].ssid, b"secret");
}

#[test]
fn full_flow_scan_auth_assoc_handshake_and_encrypted_data() {
    let mut sim = wpa2_sim();
    connect_and_authorize(&mut sim);

    // Event order: ScanDone, then Connected with both RSN IEs.
    let connected = sim.events.iter().find_map(|e| match e {
        Event::Connected { bssid, ssid, channel, ap_rsn_ie, sta_rsn_ie, roamed, aid } => {
            Some((*bssid, ssid.clone(), *channel, ap_rsn_ie.clone(), sta_rsn_ie.clone(), *roamed, *aid))
        }
        _ => None,
    });
    let (bssid, ssid, ch, ap_ie, sta_ie, roamed, aid) = connected.expect("connected");
    assert_eq!((bssid, &ssid[..], ch, roamed, aid), (AP1, &b"ecm-lab"[..], 6, false, 1));
    assert_eq!(ap_ie.unwrap(), sim.aps[0].rsn_ie());
    assert!(sta_ie.is_some());
    assert_eq!(sim.supp.state(), &HandshakeState::Completed);
    assert!(sim.supp_events.contains(&SupplicantEvent::Completed { bssid: AP1 }));
    assert_eq!(sim.ptk_installs(), 1);
    assert!(sim.installs.iter().any(|o| matches!(o, SupplicantOutput::InstallGtk { key_id: 1, .. })));

    // The STA's and AP's view of the PTK agree.
    let ap_tk = sim.aps[0].sta.as_ref().unwrap().ptk.as_ref().unwrap().tk;
    assert!(sim.installs.iter().any(|o| matches!(o, SupplicantOutput::InstallPtk { tk, .. } if *tk == ap_tk)));

    // Uplink: STA → AP → wired, encrypted on the air.
    let up = ip_frame(&HOST, &STA_MAC, b"hello from wlan0");
    let tx = sim.sta.send_ethernet(&up, sim.now).expect("authorized");
    assert!(frame::Header::parse(&tx.frame).unwrap().0.protected());
    assert!(!tx.frame.windows(16).any(|w| w == b"hello from wlan0"), "ciphertext on air");
    assert!(ecm_wifi::rate::Rate::from_code(tx.rate).is_some());
    sim.aps[0].on_rx(&tx.frame, sim.now);
    assert_eq!(sim.aps[0].wired_rx, vec![up]);

    // Downlink unicast (TK) and broadcast (GTK).
    let down = ip_frame(&STA_MAC, &HOST, b"hello from the LAN");
    sim.aps[0].downlink(&down, false);
    let bcast = ip_frame(&frame::BROADCAST, &HOST, b"broadcast");
    sim.aps[0].downlink(&bcast, false);
    sim.pump();
    assert_eq!(sim.sta_rx, vec![down, bcast]);
    let st = sim.sta.stats();
    assert_eq!(st.rx_packets, 2 + 2, "two data + M1/M3 EAPOL");

    // iw dev wlan0 link
    let li = sim.sta.link_info(sim.now).unwrap();
    assert_eq!(li.bssid, AP1);
    assert_eq!(li.freq_mhz, 2437);
    assert_eq!(li.signal_dbm, -50);
    assert!(li.authorized);
    let text = li.format_iw();
    assert!(text.contains("Connected to 02:aa:00:00:00:01"), "{text}");
    assert!(text.contains("SSID: ecm-lab"));
    assert!(text.contains("signal: -50 dBm"));
    assert!(text.contains("tx bitrate:"));
}

#[test]
fn wrong_passphrase_fails_at_m2_mic_and_installs_no_keys() {
    let mut sim = Sim::new(vec![TestAp::new(AP1, "ecm-lab", 6, Some(PASS))]);
    sim.add_network(NetworkConfig::wpa2_passphrase(b"ecm-lab", "not the password").unwrap());
    sim.start_wpa_supplicant();
    sim.run_until(3000, |s| s.events.iter().any(|e| matches!(e, Event::Disconnected { .. })));
    assert!(sim.aps[0].mic_failures >= 1, "AP detected the M2 MIC failure");
    assert_eq!(sim.aps[0].m3_sent, 0, "no M3 for a bad M2");
    assert_eq!(sim.ptk_installs(), 0);
    assert!(sim.installs.is_empty(), "no keys at all");
    assert!(!sim.sta.has_ptk());
    assert!(!sim.sta.authorized());
    let disc = sim.events.iter().find_map(|e| match e {
        Event::Disconnected { reason, .. } => Some(*reason),
        _ => None,
    });
    assert_eq!(disc, Some(DisconnectReason::Deauth(frame::REASON_4WAY_TIMEOUT)));
    assert!(sim.supp_events.contains(&SupplicantEvent::PossibleWrongKey { bssid: AP1 }));
    // Nothing but EAPOL may leave while unauthorized.
    assert!(sim.sta.send_ethernet(&ip_frame(&HOST, &STA_MAC, b"x"), sim.now).is_none());
}

#[test]
fn bad_m3_mic_is_detected_by_the_station() {
    let mut sim = wpa2_sim();
    sim.aps[0].corrupt_m3_mic = true;
    sim.aps[0].max_tries = 2;
    sim.start_wpa_supplicant();
    sim.run_until(3000, |s| s.supp_events.iter().any(|e| matches!(e, SupplicantEvent::MicFailure { message: 3, .. })));
    sim.run(500);
    assert_eq!(sim.ptk_installs(), 0);
    assert!(sim.installs.is_empty());
    assert_ne!(sim.supp.state(), &HandshakeState::Completed);
    assert!(sim.aps[0].m4_received == 0);
}

#[test]
fn rsn_ie_mismatch_in_m3_aborts_handshake() {
    let mut sim = wpa2_sim();
    let mut tkip = frame::Rsn::wpa2_psk();
    tkip.pairwise = vec![frame::SUITE_TKIP];
    sim.aps[0].m3_rsn_override = Some(tkip.to_ie());
    sim.start_wpa_supplicant();
    sim.run_until(3000, |s| s.supp_events.iter().any(|e| matches!(e, SupplicantEvent::RsnIeMismatch { .. })));
    assert_eq!(sim.ptk_installs(), 0);
    // The supplicant asked for a deauth with reason 17.
    assert!(sim
        .air_from_sta
        .iter()
        .any(|(f, _)| frame::Mgmt::parse(f).map_or(false, |m| m.body == frame::MgmtBody::Deauth { reason: 17 })));
}

#[test]
fn replayed_packet_number_is_dropped() {
    let mut sim = wpa2_sim();
    connect_and_authorize(&mut sim);
    let n_air = sim.air_from_ap.len();
    sim.aps[0].downlink(&ip_frame(&STA_MAC, &HOST, b"once"), false);
    sim.aps[0].downlink(&ip_frame(&frame::BROADCAST, &HOST, b"group once"), false);
    sim.pump();
    assert_eq!(sim.sta_rx.len(), 2);
    let captured: Vec<Vec<u8>> = sim.air_from_ap[n_air..].to_vec();
    // Replay both (unicast/TK and group/GTK) verbatim.
    for f in &captured {
        sim.rx_at_sta(f, -50, 6);
    }
    // Replay with the Retry bit cleared and a new sequence number (defeats duplicate
    // detection, so the PN check is what drops it).
    for f in &captured {
        let mut g = f.clone();
        frame::set_seq(&mut g, 0x777);
        sim.rx_at_sta(&g, -50, 6);
    }
    assert_eq!(sim.sta_rx.len(), 2, "replays never reach the stack");
    assert_eq!(sim.sta.stats().rx_dropped_replay, 4);
    // A fresh frame still passes.
    sim.aps[0].downlink(&ip_frame(&STA_MAC, &HOST, b"twice"), false);
    sim.pump();
    assert_eq!(sim.sta_rx.len(), 3);
}

#[test]
fn tampered_and_unencrypted_frames_are_dropped() {
    let mut sim = wpa2_sim();
    connect_and_authorize(&mut sim);
    sim.aps[0].downlink(&ip_frame(&STA_MAC, &HOST, b"payload"), false);
    let f = sim.aps[0].out.pop_front().unwrap();
    let mut bad = f.clone();
    let l = bad.len();
    bad[l - 12] ^= 0x40;
    sim.rx_at_sta(&bad, -50, 6);
    assert_eq!(sim.sta.stats().rx_dropped_mic, 1);
    // An unprotected IP frame injected by an attacker is refused once keys are required.
    let plain = frame::ethernet_to_data_fromds(&ip_frame(&STA_MAC, &HOST, b"plain"), &AP1).unwrap();
    sim.rx_at_sta(&plain, -50, 6);
    assert_eq!(sim.sta.stats().rx_dropped_unencrypted, 1);
    // The genuine frame (PN untouched by the failed attempt) still decrypts.
    sim.rx_at_sta(&f, -50, 6);
    assert_eq!(sim.sta_rx.len(), 1);
}

#[test]
fn deauth_from_ap_disconnects_and_auto_reconnects() {
    let mut sim = wpa2_sim();
    connect_and_authorize(&mut sim);
    sim.events.clear();
    sim.aps[0].deauth(frame::REASON_INACTIVITY);
    sim.pump();
    assert_eq!(
        sim.events.first(),
        Some(&Event::Disconnected { bssid: AP1, reason: DisconnectReason::Deauth(frame::REASON_INACTIVITY) })
    );
    assert!(!sim.sta.has_ptk(), "keys dropped with the association");
    assert!(!sim.sta.authorized());
    assert_eq!(sim.supp.state(), &HandshakeState::Disconnected);
    // Auto-reconnect after the delay, with a new handshake.
    sim.run_until(3000, |s| s.sta.authorized() && s.aps[0].sta_done());
    assert_eq!(sim.ptk_installs(), 2);
}

#[test]
fn local_disconnect_sends_deauth() {
    let mut sim = wpa2_sim();
    connect_and_authorize(&mut sim);
    sim.events.clear();
    sim.sta.disconnect(frame::REASON_DEAUTH_LEAVING, sim.now);
    sim.pump();
    assert_eq!(sim.aps[0].deauths_rx, 1);
    assert!(sim.aps[0].sta.is_none());
    assert_eq!(
        sim.events,
        vec![Event::Disconnected { bssid: AP1, reason: DisconnectReason::Local(frame::REASON_DEAUTH_LEAVING) }]
    );
    sim.run(3000);
    assert!(sim.connected_to().is_none(), "no auto-reconnect after a local disconnect");
}

#[test]
fn beacon_loss_disconnects() {
    let mut sim = wpa2_sim();
    connect_and_authorize(&mut sim);
    sim.events.clear();
    sim.auto_connect = false;
    sim.aps[0].enabled = false;
    let t = sim.run_until(5000, |s| s.events.iter().any(|e| matches!(e, Event::Disconnected { .. })));
    assert!(t >= 1800 && t <= 2200, "beacon loss after ~2 s, took {t}");
    assert_eq!(sim.events[0], Event::Disconnected { bssid: AP1, reason: DisconnectReason::BeaconLoss });
    // A connection-monitor probe was sent before giving up.
    assert!(sim
        .air_from_sta
        .iter()
        .any(|(f, _)| frame::frame_subtype(f) == frame::ST_PROBE_REQ && f[4..10] == AP1));
    // The AP comes back: we reconnect on our own.
    sim.aps[0].enabled = true;
    sim.aps[0].sta = None;
    sim.run_until(5000, |s| s.sta.authorized());
}

#[test]
fn lost_m3_is_retransmitted_and_lost_m4_does_not_reinstall_keys() {
    let mut sim = wpa2_sim();
    sim.aps[0].drop_m3 = 1;
    sim.aps[0].drop_m4 = 1;
    connect_and_authorize(&mut sim);
    assert!(sim.aps[0].m3_sent >= 3, "M3 sent {} times", sim.aps[0].m3_sent);
    assert_eq!(sim.ptk_installs(), 1, "the retransmitted M3 must not reinstall the PTK");
    // Data still works with the single installed key.
    sim.aps[0].downlink(&ip_frame(&STA_MAC, &HOST, b"after retx"), false);
    sim.pump();
    assert_eq!(sim.sta_rx.len(), 1);
}

#[test]
fn group_rekey_installs_new_gtk() {
    let mut sim = wpa2_sim();
    connect_and_authorize(&mut sim);
    sim.aps[0].rekey_group();
    sim.pump();
    assert!(sim.supp_events.contains(&SupplicantEvent::GroupRekey { key_id: 2 }));
    assert!(!sim.aps[0].sta.as_ref().unwrap().group_pending, "group M2 verified by the AP");
    sim.aps[0].downlink(&ip_frame(&frame::BROADCAST, &HOST, b"new gtk"), false);
    sim.pump();
    assert_eq!(sim.sta_rx.len(), 1);
}

#[test]
fn roams_to_stronger_ap_with_same_ssid() {
    let mut aps = vec![TestAp::new(AP1, "ecm-lab", 1, Some(PASS)), TestAp::new(AP2, "ecm-lab", 11, Some(PASS))];
    aps[0].rssi = -50;
    aps[1].rssi = -85;
    let mut sim = Sim::new(aps);
    sim.add_network(NetworkConfig::wpa2_passphrase(b"ecm-lab", PASS).unwrap());
    sim.start_wpa_supplicant();
    sim.run_until(3000, |s| s.sta.authorized());
    assert_eq!(sim.connected_to(), Some(AP1));
    // Walk away from AP1 towards AP2.
    sim.aps[0].rssi = -80;
    sim.aps[1].rssi = -45;
    sim.run_until(15_000, |s| s.connected_to() == Some(AP2) && s.sta.authorized() && s.aps[1].sta_done());
    assert!(sim.events.iter().any(|e| matches!(e, Event::Connected { bssid, roamed: true, .. } if *bssid == AP2)));
    assert_eq!(sim.aps[1].reassoc_reqs, 1);
    assert_eq!(sim.aps[1].sta.as_ref().unwrap().reassoc_from, Some(AP1));
    assert_eq!(sim.ptk_installs(), 2);
    // Traffic now flows through AP2.
    let up = ip_frame(&HOST, &STA_MAC, b"via ap2");
    let tx = sim.sta.send_ethernet(&up, sim.now).unwrap();
    assert_eq!(&tx.frame[4..10], &AP2);
    sim.aps[1].on_rx(&tx.frame, sim.now);
    assert_eq!(sim.aps[1].wired_rx, vec![up]);
    // No roam back while AP1 stays weak (hysteresis).
    let n = sim.events.len();
    sim.run(12_000);
    assert!(!sim.events[n..].iter().any(|e| matches!(e, Event::Connected { .. })));
}

#[test]
fn open_network_connects_without_handshake() {
    let mut sim = Sim::new(vec![TestAp::new(AP1, "cafe", 1, None)]);
    sim.add_network(NetworkConfig::open(b"cafe").unwrap());
    sim.start_wpa_supplicant();
    sim.run_until(2000, |s| s.sta.authorized());
    assert!(sim.supp_events.contains(&SupplicantEvent::Completed { bssid: AP1 }));
    let up = ip_frame(&HOST, &STA_MAC, b"plain");
    let tx = sim.sta.send_ethernet(&up, sim.now).unwrap();
    assert!(!frame::Header::parse(&tx.frame).unwrap().0.protected());
    sim.aps[0].on_rx(&tx.frame, sim.now);
    assert_eq!(sim.aps[0].wired_rx.len(), 1);
}

#[test]
fn connect_failures_are_reported() {
    // No such network.
    let mut sim = Sim::new(vec![TestAp::new(AP1, "cafe", 1, None)]);
    let mut p = ConnectParams::new(b"nowhere", ConnectSecurity::Open);
    p.auto_reconnect = false;
    sim.sta.connect(p, sim.now);
    sim.pump();
    sim.run_until(1000, |s| s.events.iter().any(|e| matches!(e, Event::ConnectFailed { .. })));
    assert!(sim.events.contains(&Event::ConnectFailed { bssid: None, reason: ConnectFailure::NoBss }));
    // Security mismatch: WPA2 requested, AP is open → not a candidate.
    let mut p = ConnectParams::new(b"cafe", ConnectSecurity::Wpa2Psk);
    p.auto_reconnect = false;
    sim.events.clear();
    sim.sta.connect(p, sim.now);
    sim.pump();
    sim.run_until(1000, |s| s.events.iter().any(|e| matches!(e, Event::ConnectFailed { .. })));
    // AP vanishes after the scan: auth times out after max_tries.
    let mut p = ConnectParams::new(b"cafe", ConnectSecurity::Open);
    p.auto_reconnect = false;
    sim.events.clear();
    sim.aps[0].enabled = false; // the BSS entry from the last scan is still fresh
    sim.sta.connect(p, sim.now);
    sim.pump();
    sim.run_until(1000, |s| s.events.iter().any(|e| matches!(e, Event::ConnectFailed { .. })));
    assert!(sim.events.contains(&Event::ConnectFailed { bssid: Some(AP1), reason: ConnectFailure::AuthTimeout }));
    let auths = sim.air_from_sta.iter().filter(|(f, _)| frame::frame_subtype(f) == frame::ST_AUTH).count();
    assert!(auths >= 3);
}

#[test]
fn monitor_mode_passes_everything_with_radiotap() {
    let mut aps = vec![TestAp::new(AP1, "ecm-lab", 6, Some(PASS))];
    aps[0].rssi = -61;
    let mut sim = Sim::new(aps);
    sim.sta.set_monitor(true, sim.now);
    assert!(sim.sta.mlme_mut().set_channel_manual(6));
    sim.pump();
    sim.run(250);
    assert!(sim.monitor_rx.len() >= 2, "beacons captured");
    let (info, hlen) = radiotap::parse(&sim.monitor_rx[0]).unwrap();
    assert_eq!(info.signal_dbm, Some(-61));
    assert_eq!(info.freq, Some(2437));
    assert_eq!(info.rate, Some(108));
    let f = &sim.monitor_rx[0][hlen..].to_vec();
    assert_eq!(frame::frame_subtype(f), frame::ST_BEACON);
    // Data frames from a foreign BSS are captured too (no decryption in the kernel).
    sim.rx_at_sta(&frame::ethernet_to_data_fromds(&ip_frame(&HOST, &AP2, b"x"), &AP2).unwrap(), -70, 6);
    assert_eq!(sim.sta_rx.len(), 0);
    // Injection: radiotap rate is honoured.
    let rt = radiotap::encapsulate(f, 0, 0x85, 6, 0, false);
    let tx = sim.sta.inject(&rt).unwrap();
    assert_eq!(tx.rate, 0x85);
    assert_eq!(&tx.frame, f);
    // Back to managed: connecting works again.
    sim.sta.set_monitor(false, sim.now);
    sim.add_network(NetworkConfig::wpa2_passphrase(b"ecm-lab", PASS).unwrap());
    sim.start_wpa_supplicant();
    sim.run_until(3000, |s| s.sta.authorized());
}

#[test]
fn bssid_lock_is_respected() {
    let mut aps = vec![TestAp::new(AP1, "ecm-lab", 1, Some(PASS)), TestAp::new(AP2, "ecm-lab", 6, Some(PASS))];
    aps[0].rssi = -40;
    aps[1].rssi = -75;
    let mut sim = Sim::new(aps);
    let mut n = NetworkConfig::wpa2_passphrase(b"ecm-lab", PASS).unwrap();
    n.bssid = Some(AP2);
    sim.add_network(n);
    sim.start_wpa_supplicant();
    sim.run_until(3000, |s| s.sta.authorized());
    assert_eq!(sim.connected_to(), Some(AP2));
}

/// A locked BSSID with a weak signal never roams, so poll must not keep
/// returning a roam-scan deadline that is already past (the kernel would spin).
#[test]
fn weak_locked_bssid_never_returns_a_past_deadline() {
    let mut aps = vec![TestAp::new(AP1, "ecm-lab", 1, Some(PASS)), TestAp::new(AP2, "ecm-lab", 6, Some(PASS))];
    aps[0].rssi = -40;
    aps[1].rssi = -75; // below the -70 dBm roam threshold
    let mut sim = Sim::new(aps);
    let mut n = NetworkConfig::wpa2_passphrase(b"ecm-lab", PASS).unwrap();
    n.bssid = Some(AP2);
    sim.add_network(n);
    sim.start_wpa_supplicant();
    sim.run_until(3000, |s| s.sta.authorized());
    assert_eq!(sim.connected_to(), Some(AP2));
    // Let the roam-scan interval lapse several times over.
    sim.run(30_000);
    let now = sim.now;
    if let Some(deadline) = sim.sta.poll(now) {
        assert!(deadline > now, "deadline {deadline} is not after now {now}");
    }
    assert_eq!(sim.connected_to(), Some(AP2), "still on the locked BSSID");
}

#[test]
fn link_bitrate_follows_tx_status_feedback() {
    let mut sim = wpa2_sim();
    connect_and_authorize(&mut sim);
    let up = ip_frame(&HOST, &STA_MAC, b"bulk");
    let feed = |sim: &mut Sim, ms: u64, ok_below_kbps: u32| {
        for _ in 0..ms {
            sim.now += 1;
            for _ in 0..2 {
                let tx = sim.sta.send_ethernet(&up, sim.now).unwrap();
                let kbps = ecm_wifi::rate::Rate::from_code(tx.rate).unwrap().kbps;
                let ok = kbps <= ok_below_kbps;
                sim.sta.tx_status(tx.rate, if ok { 1 } else { 4 }, ok, sim.now);
            }
        }
    };
    feed(&mut sim, 3000, u32::MAX);
    let fast = sim.sta.rate_control().unwrap().best_rate();
    assert_eq!(fast.kbps, 65_000, "HT MCS 7 with an HT AP");
    feed(&mut sim, 3000, 13_000);
    let li = sim.sta.link_info(sim.now).unwrap();
    assert!(sim.sta.rate_control().unwrap().best_rate().kbps <= 13_000);
    assert!(li.format_iw().contains("tx bitrate:"));
    assert!(sim.sta.stats().tx_failed > 0 && sim.sta.stats().tx_retries > 0);
}
