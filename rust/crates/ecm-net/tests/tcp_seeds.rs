//! Robustness sweep: byte-exact TCP transfer, close and socket cleanup under 5-15% loss
//! and 30% reordering, for many PRNG seeds.
mod common;
use common::*;
use ecm_net::*;
#[test]
fn tcp_transfer_and_teardown_across_60_seeds_high_loss() {
    for seed in 200..260u64 {
        let mut net = two_hosts(seed);
        net.capture_on = false;
        net.segments[0].loss = if seed % 2 == 0 { 0.05 } else { 0.15 };
        net.segments[0].reorder = 0.3;
        let l = net.s(1).tcp_listen(sa("0.0.0.0", 80), 8).unwrap();
        let now = net.now;
        let c = net.s(0).tcp_connect(sa("10.0.0.2", 80), now).unwrap();
        let mut srv = None;
        assert!(net.run(600_000, |n| { if srv.is_none() { srv = n.s(1).tcp_accept(l).unwrap(); } srv.is_some() }), "seed {seed} accept");
        let s = srv.unwrap();
        let data = Rng(seed).bytes(150_000 + seed as usize * 101);
        let (mut sent, mut closed, mut got) = (0, false, Vec::new());
        let mut buf = vec![0u8; 7000];
        let ok = net.run(3_600_000, |n| {
            let now = n.now;
            while sent < data.len() { match n.s(0).tcp_send(c, &data[sent..], now) { Ok(k) => sent += k, Err(NetError::WouldBlock) => break, Err(e) => panic!("seed {seed} send {e:?}") } }
            if sent == data.len() && !closed { n.s(0).tcp_close(c, now); closed = true; }
            loop { match n.s(1).tcp_recv(s, &mut buf) { Ok(0) => return true, Ok(k) => got.extend_from_slice(&buf[..k]), Err(NetError::WouldBlock) => return false, Err(e) => panic!("seed {seed} recv {e:?}") } }
        });
        assert!(ok && got == data, "seed {seed}: {} of {}", got.len(), data.len());
        let now = net.now;
        net.s(1).tcp_close(s, now); net.s(1).tcp_close(l, now);
        net.advance(200_000);
        assert_eq!(net.stacks[0].tcp_list().count() + net.stacks[1].tcp_list().count(), 0, "seed {seed} leaked sockets");
    }
}
