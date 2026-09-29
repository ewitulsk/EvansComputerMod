//! Deterministic fuzz-style tests: random and mutated frames into `handle_frame`,
//! and garbage arguments into every API. Success = no panic, stack still works.

mod common;

use common::*;
use ecm_net::checksum::{internet_checksum, pseudo_header_checksum};
use ecm_net::icmp::{IcmpPacket, ICMP_ECHO_REQUEST};
use ecm_net::*;

const A: usize = 0;
const B: usize = 1;

/// Recompute IPv4/TCP/UDP/ICMP checksums in place (lenient: fixes whatever parses),
/// so mutations reach the upper layers instead of dying at the checksum.
fn fix_checksums(f: &mut [u8]) {
    let mut off = 14;
    if f.len() >= 18 && f[12] == 0x81 && f[13] == 0x00 {
        off = 18;
    }
    if f.len() < off + 20 || f[off] >> 4 != 4 {
        return;
    }
    let ihl = (f[off] & 0xf) as usize * 4;
    if ihl < 20 || off + ihl > f.len() {
        return;
    }
    f[off + 10] = 0;
    f[off + 11] = 0;
    let c = internet_checksum(&f[off..off + ihl]);
    f[off + 10..off + 12].copy_from_slice(&c.to_be_bytes());
    let total = (u16::from_be_bytes([f[off + 2], f[off + 3]]) as usize).min(f.len() - off);
    if total < ihl {
        return;
    }
    let src = Ipv4Addr::from_bytes(&f[off + 12..off + 16]);
    let dst = Ipv4Addr::from_bytes(&f[off + 16..off + 20]);
    let proto = f[off + 9];
    let seg = &mut f[off + ihl..off + total];
    let pos = match proto {
        6 if seg.len() >= 20 => 16,
        17 if seg.len() >= 8 => 6,
        1 if seg.len() >= 4 => 2,
        _ => return,
    };
    seg[pos] = 0;
    seg[pos + 1] = 0;
    let c = if proto == 1 { internet_checksum(seg) } else { pseudo_header_checksum(&src, &dst, proto, seg) };
    seg[pos..pos + 2].copy_from_slice(&c.to_be_bytes());
}

fn mutate(rng: &mut Rng, base: &[u8]) -> Vec<u8> {
    let mut f = base.to_vec();
    for _ in 0..1 + rng.below(4) {
        let len = f.len().max(1);
        match rng.below(8) {
            0 => {
                let i = rng.below(len as u64) as usize;
                if i < f.len() {
                    f[i] ^= 1 << rng.below(8);
                }
            }
            1 => {
                let i = rng.below(len as u64) as usize;
                if i < f.len() {
                    f[i] = rng.next_u64() as u8;
                }
            }
            2 => f.truncate(rng.below(len as u64 + 1) as usize),
            3 => {
                let n = rng.below(64) as usize;
                let extra = rng.bytes(n);
                f.extend_from_slice(&extra);
            }
            4 | 5 => {
                // interesting 16-bit values at header-ish offsets (lengths, ports, flags)
                let i = rng.below(len.min(80) as u64) as usize;
                let v: u16 = [0, 1, 4, 5, 7, 8, 19, 20, 0x45, 0x0fff, 0x8000, 0xffff, f.len() as u16][rng.below(13) as usize];
                if i + 2 <= f.len() {
                    f[i..i + 2].copy_from_slice(&v.to_be_bytes());
                }
            }
            6 => {
                // insert an 802.1Q tag with a random VID
                if f.len() >= 14 {
                    let vid = [0u16, 1, 10, 4095][rng.below(4) as usize];
                    f.splice(12..12, [0x81, 0x00, (vid >> 8) as u8, vid as u8]);
                }
            }
            _ => {
                // DNS-ish: plant compression pointers anywhere
                let i = rng.below(len as u64) as usize;
                if i + 2 <= f.len() {
                    f[i] = 0xC0 | rng.below(0x40) as u8;
                    f[i + 1] = rng.next_u64() as u8;
                }
            }
        }
    }
    if rng.chance(0.6) {
        fix_checksums(&mut f);
    }
    f
}

/// Build two stacks with live state (established TCP, listener, UDP, ICMP, DNS)
/// and return the frames seen on the wire as a corpus.
fn setup(seed: u64) -> (Net, Vec<Vec<u8>>, Vec<SocketHandle>) {
    let mut net = two_hosts(seed);
    net.s(A).set_dns_server(ip("10.0.0.2"));
    let l = net.s(B).tcp_listen(sa("0.0.0.0", 80), 16).unwrap();
    let u = net.s(B).udp_bind(sa("0.0.0.0", 53)).unwrap();
    let ic = net.s(B).icmp_open().unwrap();
    let now = net.now;
    let c = net.s(A).tcp_connect(sa("10.0.0.2", 80), now).unwrap();
    let mut srv = None;
    net.run(5000, |n| {
        srv = srv.or_else(|| n.s(B).tcp_accept(l).unwrap());
        srv.is_some()
    });
    let srv = srv.unwrap();
    let now = net.now;
    net.s(A).tcp_send(c, &[7u8; 3000], now).unwrap();
    net.s(B).tcp_send(srv, b"hello", now).unwrap();
    let ua = net.s(A).udp_bind(sa("0.0.0.0", 0)).unwrap();
    net.s(A).udp_send_to(ua, sa("10.0.0.2", 53), b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x01a\x00\x00\x01\x00\x01", now).unwrap();
    let _q = net.s(A).dns_query("fuzz.example", now).unwrap();
    let ia = net.s(A).icmp_open().unwrap();
    net.s(A).icmp_send(ia, ip("10.0.0.2"), &IcmpPacket::build_echo(ICMP_ECHO_REQUEST, 1, 2, b"abc"), now).unwrap();
    net.advance(50);
    let now = net.now;
    let c2 = net.s(A).tcp_connect(sa("10.0.0.2", 80), now).unwrap();
    net.advance(50);
    let now = net.now;
    net.s(A).tcp_close(c2, now);
    net.advance(50);
    let corpus: Vec<Vec<u8>> = net.capture.iter().map(|c| c.frame.clone()).collect();
    assert!(corpus.len() > 15);
    (net, corpus, vec![l, u, ic, c, srv, ua, ia])
}

#[test]
fn fuzz_100k_frames_no_panic() {
    let (mut net, corpus, handles) = setup(4242);
    let mut rng = Rng(0xF00D);
    let mut now = net.now;
    for i in 0..100_000u32 {
        let frame = if rng.chance(0.3) {
            let n = rng.below(1600) as usize;
            let mut f = rng.bytes(n);
            if f.len() >= 14 && rng.chance(0.5) {
                // aim at B's MAC with a plausible ethertype
                f[..6].copy_from_slice(&mac(2).0);
                let et: [u8; 2] = [[0x08, 0x00], [0x08, 0x06], [0x81, 0x00]][rng.below(3) as usize];
                f[12..14].copy_from_slice(&et);
                if rng.chance(0.5) {
                    fix_checksums(&mut f);
                }
            }
            f
        } else {
            let base = &corpus[rng.below(corpus.len() as u64) as usize];
            mutate(&mut rng, base)
        };
        let target = if rng.chance(0.8) { B } else { A };
        net.stacks[target].handle_frame(0, &frame, now);
        if i % 64 == 0 {
            now += rng.below(3000) as i64;
            for s in net.stacks.iter_mut() {
                let _ = s.poll(now);
                while s.pop_tx().is_some() {}
            }
            // exercise the API against whatever state the fuzz produced
            let h = handles[rng.below(handles.len() as u64) as usize];
            let s = &mut net.stacks[B];
            let mut buf = [0u8; 2048];
            let _ = s.tcp_accept(h);
            let _ = s.tcp_recv(h, &mut buf);
            let _ = s.tcp_send(h, b"x", now);
            let _ = s.udp_recv_from(h, &mut buf);
            let _ = s.icmp_recv(h, &mut buf);
            let _ = s.tcp_can_read(h);
            let _ = s.tcp_can_write(h);
        }
        if i % 5000 == 0 {
            // the TX queue is bounded no matter what we feed in
            for s in net.stacks.iter_mut() {
                let mut n = 0;
                while s.pop_tx().is_some() {
                    n += 1;
                }
                assert!(n <= TX_QUEUE_LEN);
            }
        }
    }
    // The mutations reached every layer.
    let st = net.stacks[B].stats();
    let ifs = net.stacks[B].iface(0).unwrap().stats;
    eprintln!("{st:?}
{ifs:?}");
    assert!(st.ip_rx_bad > 0 && st.ip_rx_fragments_dropped > 0 && st.tcp_rx_bad > 0 && st.udp_rx_bad > 0 && st.icmp_rx_bad > 0);
    assert!(st.tcp_rst_sent > 0 && st.udp_rx_no_socket > 0 && ifs.rx_errors > 0 && ifs.rx_dropped > 0);
    // Still functional afterwards: a fresh ping from A to B works.
    net.now = now + 100_000;
    for s in net.stacks.iter_mut() {
        let _ = s.poll(net.now);
        while s.pop_tx().is_some() {}
    }
    let ic = net.s(A).icmp_open().unwrap();
    let echo = IcmpPacket::build_echo(ICMP_ECHO_REQUEST, 77, 1, b"alive");
    let t = net.now;
    net.s(A).icmp_send(ic, ip("10.0.0.2"), &echo, t).unwrap();
    let mut buf = [0u8; 64];
    assert!(net.run(5000, |n| matches!(n.s(A).icmp_recv(ic, &mut buf), Ok(Some(_)))));
}

#[test]
fn every_prefix_of_every_valid_frame() {
    let (mut net, corpus, _) = setup(7);
    let now = net.now;
    for f in &corpus {
        for n in 0..=f.len() {
            net.stacks[B].handle_frame(0, &f[..n], now);
            net.stacks[A].handle_frame(0, &f[..n], now);
        }
    }
    for s in net.stacks.iter_mut() {
        let _ = s.poll(now + 1);
    }
}

#[test]
fn garbage_api_arguments() {
    let mut rng = Rng(99);
    let mut s = Stack::new(StackConfig { seed: 3 });
    let e0 = s.add_interface("eth0", mac(1)).unwrap();
    s.configure_addr(e0, ip("10.0.0.1"), 24, 0);
    for i in 0..20_000i64 {
        let h = SocketHandle::from_raw(rng.next_u64() as u32);
        let a = SocketAddr::new(Ipv4Addr::from_u32(rng.next_u64() as u32), rng.next_u64() as u16);
        let idx = rng.below(40) as usize;
        let n = rng.below(2000) as usize;
        let data = rng.bytes(n);
        let mut buf = vec![0u8; rng.below(100) as usize];
        let name: String = (0..rng.below(300)).map(|_| (rng.below(96) as u8 + 32) as char).collect();
        match rng.below(26) {
            0 => {
                let _ = s.tcp_listen(a, rng.below(1000) as usize);
            }
            1 => {
                let _ = s.tcp_connect(a, i);
            }
            2 => {
                let _ = s.tcp_send(h, &data, i);
            }
            3 => {
                let _ = s.tcp_recv(h, &mut buf);
            }
            4 => s.tcp_close(h, i),
            5 => s.tcp_abort(h),
            6 => {
                let _ = s.tcp_shutdown_write(h, i);
            }
            7 => {
                let _ = s.udp_bind(a);
            }
            8 => {
                let _ = s.udp_send_to(h, a, &data, i);
            }
            9 => {
                let _ = s.udp_recv_from(h, &mut buf);
            }
            10 => s.udp_close(h),
            11 => {
                let _ = s.icmp_open();
            }
            12 => {
                let _ = s.icmp_send(h, a.ip, &data, i);
            }
            13 => {
                let _ = s.dns_query(&name, i);
            }
            14 => {
                let _ = s.dns_poll(DnsHandle::from_raw(rng.next_u64() as u32));
            }
            15 => s.configure_addr(idx, a.ip, rng.below(40) as u8, i),
            16 => s.set_vlan(idx, Some(rng.next_u64() as u16)),
            17 => {
                let _ = s.add_route(a.ip, rng.below(40) as u8, Ipv4Addr::from_u32(rng.next_u64() as u32), idx);
            }
            18 => {
                let _ = s.del_route(a.ip, rng.below(40) as u8);
            }
            19 => {
                let _ = s.add_interface(&name, mac(rng.below(255) as u8));
            }
            20 => {
                if rng.chance(0.1) {
                    s.remove_interface(idx)
                }
            }
            21 => s.set_link(idx, rng.chance(0.7), i),
            22 => s.set_admin_up(idx, rng.chance(0.7), i),
            23 => s.set_dns_server(a.ip),
            24 => {
                let _ = (s.tcp_state(h), s.tcp_local_addr(h), s.tcp_peer_addr(h), s.udp_local_addr(h), s.neighbors(idx).count());
            }
            _ => {
                let _ = s.poll(i);
            }
        }
        if i % 16 == 0 {
            while let Some((iface, f)) = s.pop_tx() {
                // reflect some output back in (looks like a hub echoing us)
                if rng.chance(0.2) {
                    s.handle_frame(iface, &f, i);
                }
            }
        }
    }
    let _ = s.poll(1_000_000);
    assert!(s.routes().len() <= MAX_ROUTES);
    assert!(s.iface_count() <= MAX_INTERFACES);
}
