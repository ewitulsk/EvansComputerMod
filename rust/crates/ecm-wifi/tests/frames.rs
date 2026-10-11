//! Codec round trips: MAC header, IEs, management frames, data ↔ Ethernet, EAPOL-Key,
//! KDEs, radiotap, FCS, channel maps.

use ecm_wifi::crypto::{self, ReplayCounters, TxPn};
use ecm_wifi::eapol::{self, KeyFrame};
use ecm_wifi::frame::{self, BeaconBody, BssInfo, Header, Mgmt, MgmtBody, Rsn, Security};
use ecm_wifi::radiotap;

const A: [u8; 6] = [0x02, 1, 2, 3, 4, 5];
const B: [u8; 6] = [0x02, 6, 7, 8, 9, 10];
const C: [u8; 6] = [0x02, 11, 12, 13, 14, 15];

fn all_mgmt_bodies() -> Vec<MgmtBody> {
    let mut ies = Vec::new();
    frame::push_ie(&mut ies, frame::IE_SSID, b"net");
    frame::push_rates(&mut ies, &[0x82, 0x84, 0x8b, 0x96, 12, 18, 24, 36, 48, 72, 96, 108]);
    vec![
        MgmtBody::Beacon(BeaconBody { timestamp: 0x0102_0304_0506_0708, interval: 100, cap: 0x0411, ies: ies.clone() }),
        MgmtBody::ProbeResp(BeaconBody { timestamp: 7, interval: 100, cap: 1, ies: ies.clone() }),
        MgmtBody::ProbeReq { ies: ies.clone() },
        MgmtBody::Auth { algo: 0, seq: 2, status: 0, ies: Vec::new() },
        MgmtBody::AssocReq { cap: 0x0431, listen: 10, ies: ies.clone() },
        MgmtBody::ReassocReq { cap: 0x0431, listen: 10, current_ap: C, ies: ies.clone() },
        MgmtBody::AssocResp { cap: 0x0411, status: 0, aid: 5, ies: Vec::new() },
        MgmtBody::ReassocResp { cap: 0x0411, status: 17, aid: 2007, ies: ies.clone() },
        MgmtBody::Disassoc { reason: 8 },
        MgmtBody::Deauth { reason: 15 },
    ]
}

#[test]
fn management_frames_round_trip() {
    for (i, body) in all_mgmt_bodies().into_iter().enumerate() {
        let mut m = Mgmt::new(A, B, C, body);
        m.hdr.seq = 100 + i as u16;
        let bytes = m.to_bytes();
        assert_eq!(frame::header_len(&bytes), Some(24));
        let back = Mgmt::parse(&bytes).unwrap();
        assert_eq!(back, m, "frame {i}");
        assert_eq!(back.to_bytes(), bytes);
    }
}

#[test]
fn deauth_wire_format_is_exact() {
    let mut m = Mgmt::new(A, B, C, MgmtBody::Deauth { reason: 7 });
    m.hdr.seq = 0x123;
    let b = m.to_bytes();
    let mut expect = vec![0xc0, 0x00, 0x00, 0x00];
    expect.extend_from_slice(&A);
    expect.extend_from_slice(&B);
    expect.extend_from_slice(&C);
    expect.extend_from_slice(&[0x30, 0x12, 0x07, 0x00]);
    assert_eq!(b, expect);
}

#[test]
fn header_lengths_qos_and_four_address() {
    let mut h = Header::new(frame::TYPE_DATA, frame::ST_QOS_DATA, frame::FL_FROM_DS, A, B, C);
    h.qos = Some(0x0005);
    let mut v = Vec::new();
    h.write(&mut v);
    assert_eq!(frame::header_len(&v), Some(26));
    assert_eq!(frame::qos_control(&v), Some(5));
    assert_eq!(frame::tid(&v), 5);
    let (p, l) = Header::parse(&v).unwrap();
    assert_eq!((p, l), (h, 26));

    let mut h4 = Header::new(frame::TYPE_DATA, frame::ST_DATA, 0, A, B, C);
    h4.addr4 = Some([9; 6]);
    let mut v4 = Vec::new();
    h4.write(&mut v4);
    assert!(frame::has_addr4(&v4));
    assert_eq!(frame::header_len(&v4), Some(30));
    assert_eq!(Header::parse(&v4).unwrap().0.bssid(), None);
    // Control frames are not parsed as MAC headers.
    assert_eq!(frame::header_len(&[0xd4, 0, 0, 0, 1, 2, 3, 4, 5, 6]), None);
    assert_eq!(frame::header_len(&[0x08, 0x01]), None, "truncated");
}

#[test]
fn rsn_ie_round_trip_and_defaults() {
    let r = Rsn::wpa2_psk();
    let ie = r.to_ie();
    assert_eq!(ie, frame::find_ie_raw(&ie, frame::IE_RSN).unwrap());
    assert_eq!(hexs(&ie), "30140100000fac040100000fac040100000fac020000");
    assert_eq!(Rsn::parse(&ie[2..]).unwrap(), r);
    assert!(r.supports_psk_ccmp());
    // Version-only RSN IE: everything defaults to CCMP + 802.1X.
    let d = Rsn::parse(&[1, 0]).unwrap();
    assert_eq!(d.akm, vec![frame::SUITE_AKM_8021X]);
    assert!(!d.supports_psk_ccmp());
    assert!(Rsn::parse(&[2, 0]).is_none(), "unknown version");
    assert!(Rsn::parse(&[1, 0, 0, 0x0f, 0xac, 4, 5, 0]).is_none(), "truncated suite list");
}

#[test]
fn beacon_to_bss_info() {
    let mut ies = Vec::new();
    frame::push_ie(&mut ies, frame::IE_SSID, b"ecm");
    frame::push_rates(&mut ies, &[0x82, 0x84, 0x8b, 0x96, 12, 18, 24, 36, 48, 72]);
    frame::push_ie(&mut ies, frame::IE_DS_PARAMS, &[11]);
    ies.extend_from_slice(&Rsn::wpa2_psk().to_ie());
    frame::push_ie(&mut ies, frame::IE_HT_CAP, &frame::ht_cap_body());
    frame::push_ie(&mut ies, 0xdd, &[0, 0x50, 0xf2, 2, 1, 1]); // WMM vendor IE ignored
    let bb = BeaconBody { timestamp: 0, interval: 100, cap: frame::CAP_ESS | frame::CAP_PRIVACY, ies };
    let m = Mgmt::new(frame::BROADCAST, A, A, MgmtBody::Beacon(bb.clone()));
    let parsed = Mgmt::parse(&m.to_bytes()).unwrap();
    let MgmtBody::Beacon(pb) = parsed.body else { panic!() };
    let info = BssInfo::from_beacon(A, &pb, 11, -63, 5000);
    assert_eq!(info.ssid, b"ecm");
    assert_eq!(info.channel, 11);
    assert_eq!(info.rates.len(), 10, "supported + extended");
    assert_eq!(info.ht_mcs, 0xff);
    assert_eq!(info.security(), Security::Wpa2Psk);
    assert_eq!(info.rssi, -63);
    assert!(!info.hidden());
    // WEP-style privacy without RSN is listed as unsupported.
    let mut wep = info.clone();
    wep.rsn = None;
    assert_eq!(wep.security(), Security::Unsupported);
}

#[test]
fn data_frames_convert_to_and_from_ethernet() {
    let eth = frame::ethernet(&B, &A, 0x0800, b"ip packet bytes");
    let up = frame::ethernet_to_data_tods(&eth, &C).unwrap();
    let (h, hl) = Header::parse(&up).unwrap();
    assert!(h.to_ds() && !h.from_ds());
    assert_eq!((h.addr1, h.addr2, h.addr3), (C, A, B));
    assert_eq!(&up[hl..hl + 8], &[0xaa, 0xaa, 0x03, 0, 0, 0, 0x08, 0x00]);
    assert_eq!(frame::data_to_ethernet(&up).unwrap(), eth);

    let down = frame::ethernet_to_data_fromds(&eth, &C).unwrap();
    let (h, _) = Header::parse(&down).unwrap();
    assert_eq!((h.addr1, h.addr2, h.addr3), (B, C, A));
    assert_eq!(frame::data_to_ethernet(&down).unwrap(), eth);

    // QoS data with a TID still converts.
    let mut q = Header::new(frame::TYPE_DATA, frame::ST_QOS_DATA, frame::FL_FROM_DS, B, C, A);
    q.qos = Some(6);
    let mut qf = Vec::new();
    q.write(&mut qf);
    qf.extend_from_slice(&frame::LLC_SNAP);
    qf.extend_from_slice(&eth[12..]);
    assert_eq!(frame::data_to_ethernet(&qf).unwrap(), eth);
    // Null function frames carry nothing.
    let mut n = Vec::new();
    Header::new(frame::TYPE_DATA, frame::ST_NULL, frame::FL_TO_DS, C, A, C).write(&mut n);
    assert!(frame::data_to_ethernet(&n).is_none());
    // EAPOL keeps its EtherType.
    let e = frame::ethernet(&B, &A, frame::ETHERTYPE_EAPOL, &[2, 3, 0, 0]);
    assert_eq!(frame::ethertype(&frame::data_to_ethernet(&frame::ethernet_to_data_tods(&e, &C).unwrap()).unwrap()), Some(0x888e));
}

#[test]
fn fcs_and_channels() {
    let mut f = b"123456789".to_vec();
    assert_eq!(frame::fcs(&f), 0xcbf4_3926, "CRC-32 check value");
    frame::append_fcs(&mut f);
    assert_eq!(frame::strip_fcs(&f).unwrap(), b"123456789");
    f[0] ^= 1;
    assert!(frame::strip_fcs(&f).is_none());
    for (ch, mhz) in [(1u8, 2412u16), (6, 2437), (11, 2462), (13, 2472), (14, 2484), (36, 5180), (165, 5825)] {
        assert_eq!(frame::channel_to_freq(ch), mhz);
        assert_eq!(frame::freq_to_channel(mhz), Some(ch));
    }
}

#[test]
fn eapol_key_frame_round_trip_and_kdes() {
    let mut k = KeyFrame::new(2, eapol::KI_VERSION_AES_SHA1 | eapol::KI_PAIRWISE | eapol::KI_ACK);
    k.key_len = 16;
    k.replay_counter = 0x0102030405060708;
    k.nonce = [0x5a; 32];
    k.rsc = 0x1122;
    k.key_data = vec![1, 2, 3];
    let b = k.to_bytes();
    assert_eq!(b.len(), 99 + 3);
    assert_eq!(&b[..4], &[2, 3, 0, 98]);
    assert_eq!(KeyFrame::parse(&b).unwrap(), k);
    // Ethernet padding after the body is ignored.
    let mut padded = b.clone();
    padded.extend_from_slice(&[0; 10]);
    assert_eq!(KeyFrame::parse(&padded).unwrap(), k);
    assert!(KeyFrame::parse(&b[..90]).is_none());
    assert!(eapol::is_pairwise_key_frame(&b));

    let mut kd = Rsn::wpa2_psk().to_ie();
    kd.extend_from_slice(&eapol::gtk_kde(2, true, &[0x77; 16]));
    let padded = eapol::pad_key_data(kd.clone());
    assert_eq!(padded.len() % 8, 0);
    assert_eq!(padded[kd.len()], 0xdd);
    let kek = [9u8; 16];
    let wrapped = crypto::aes_wrap(&kek, &padded).unwrap();
    let parsed = eapol::parse_key_data(&crypto::aes_unwrap(&kek, &wrapped).unwrap());
    assert_eq!(parsed.rsn_ie.unwrap(), Rsn::wpa2_psk().to_ie());
    let g = parsed.gtk.unwrap();
    assert_eq!((g.key_id, g.tx, g.gtk), (2, true, vec![0x77; 16]));
    // Short key data is padded to the 16-byte minimum.
    assert_eq!(eapol::pad_key_data(vec![1, 2, 3]).len(), 16);
}

#[test]
fn radiotap_round_trip_legacy_and_ht() {
    let rt = radiotap::write(123_456, 108, 6, -55, true);
    let (i, l) = radiotap::parse(&rt).unwrap();
    assert_eq!(l, rt.len());
    assert_eq!(i.tsft, Some(123_456));
    assert_eq!(i.rate, Some(108));
    assert_eq!(i.freq, Some(2437));
    assert_eq!(i.signal_dbm, Some(-55));
    assert_eq!(i.flags, Some(radiotap::F_FCS));
    let ht = radiotap::write(1, 0x87, 36, -80, false);
    let (i, _) = radiotap::parse(&ht).unwrap();
    assert_eq!(i.rate, Some(0x87));
    assert_eq!(i.freq, Some(5180));
    // A foreign header with an extended bitmap and fields we skip (antenna noise,
    // antenna index, RX flags) before an unknown namespace.
    let mut h = vec![0u8, 0, 0, 0];
    h.extend_from_slice(&(0x8000_0000u32 | (1 << 2) | (1 << 5) | (1 << 6) | (1 << 11) | (1 << 14)).to_le_bytes());
    h.extend_from_slice(&0u32.to_le_bytes()); // second presence word
    h.push(22); // rate
    h.push((-40i8) as u8);
    h.push((-95i8) as u8);
    h.push(1); // antenna
    h.extend_from_slice(&0u16.to_le_bytes()); // rx flags (2-aligned at 16)
    let len = h.len() as u16;
    h[2..4].copy_from_slice(&len.to_le_bytes());
    h.extend_from_slice(&[0x80, 0]);
    let (i, l) = radiotap::parse(&h).unwrap();
    assert_eq!((i.rate, i.signal_dbm, l), (Some(22), Some(-40), len as usize));
    assert!(radiotap::parse(&[1, 0, 8, 0, 0, 0, 0, 0]).is_none(), "bad version");
}

#[test]
fn replay_counters_and_tx_pn() {
    let mut r = ReplayCounters::new();
    assert!(r.accept(0, 1));
    assert!(!r.accept(0, 1));
    assert!(r.accept(0, 5));
    assert!(!r.accept(0, 3));
    assert!(r.accept(3, 2), "TIDs are independent");
    let g = ReplayCounters::with_start(10);
    assert!(!g.check(0, 10));
    assert!(g.check(0, 11));
    let mut pn = TxPn::new();
    assert_eq!(pn.next(), Some(1));
    assert_eq!(pn.next(), Some(2));
}

fn hexs(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
