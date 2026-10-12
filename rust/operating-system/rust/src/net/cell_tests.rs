//! `wwan0` in the real kernel dispatcher: a fake modem bridged by an ideal
//! "tower" onto a second kernel's `eth0`. The interface appears with the module,
//! follows the modem's carrier, carries ARP/UDP/TCP both ways, keeps a TCP
//! connection across a short carrier drop (a handover) and is re-created with
//! the new MAC when a different modem is fitted.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use ecm_net::types::{Ipv4Addr, SocketAddr};
use ecm_net::TcpState;

use super::{CellModem, IFNAME, POLL_MS};
use crate::net::{Net, Nics};

#[derive(Default)]
struct Air {
    present: usize,
    link: bool,
    mac: [u8; 6],
    /// Frames the modem sent (towards the server's NIC).
    up: VecDeque<Vec<u8>>,
    /// Frames for the modem (from the server's NIC).
    down: VecDeque<Vec<u8>>,
    /// Frames the server's NIC sent.
    server_out: VecDeque<Vec<u8>>,
}

struct FakeModem(Rc<RefCell<Air>>);

impl CellModem for FakeModem {
    fn present(&self) -> usize {
        self.0.borrow().present
    }
    fn mac(&self) -> Option<[u8; 6]> {
        let a = self.0.borrow();
        (a.present > 0).then_some(a.mac)
    }
    fn link(&self) -> bool {
        self.0.borrow().link
    }
    fn tx(&mut self, frame: &[u8]) -> bool {
        let mut a = self.0.borrow_mut();
        if !a.link {
            return false;
        }
        a.up.push_back(frame.to_vec());
        true
    }
    fn rx(&mut self, buf: &mut [u8]) -> Option<usize> {
        let f = self.0.borrow_mut().down.pop_front()?;
        let n = f.len().min(buf.len());
        buf[..n].copy_from_slice(&f[..n]);
        Some(n)
    }
}

/// The server's single NIC, cabled to the tower.
struct ServerNic(Rc<RefCell<Air>>);

const SERVER_MAC: [u8; 6] = [0x02, 0x5e, 0, 0, 0, 1];

impl Nics for ServerNic {
    fn count(&self) -> usize {
        1
    }
    fn mac(&self, _: usize) -> Option<[u8; 6]> {
        Some(SERVER_MAC)
    }
    fn tx(&mut self, _: usize, frame: &[u8]) {
        self.0.borrow_mut().server_out.push_back(frame.to_vec());
    }
    fn rx(&mut self, buf: &mut [u8]) -> Option<(usize, usize)> {
        let f = self.0.borrow_mut().up.pop_front()?;
        let n = f.len().min(buf.len());
        buf[..n].copy_from_slice(&f[..n]);
        Some((0, n))
    }
    fn set_promiscuous(&mut self, _: usize, _: bool) {}
    fn carrier(&self, _: usize) -> bool {
        true
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

struct World {
    now: i64,
    air: Rc<RefCell<Air>>,
    phone: Net,
    server: Net,
}

impl World {
    fn new() -> Self {
        let air = Rc::new(RefCell::new(Air { mac: [0x02, 0xce, 0x11, 0, 0, 1], ..Default::default() }));
        let phone = Net::with_nics(Box::new(NoNics), 1, 0).with_cell(Box::new(FakeModem(air.clone())), 0);
        let mut server = Net::with_nics(Box::new(ServerNic(air.clone())), 2, 0);
        server.stack.configure_addr(0, Ipv4Addr([10, 9, 0, 1]), 24, 0);
        Self { now: 0, air, phone, server }
    }

    /// One 10 ms step: both kernels poll, frames cross the ideal tower (only while attached).
    fn step(&mut self) {
        self.now += 10;
        let now = self.now;
        self.phone.poll(now);
        self.phone.rx(now);
        self.server.rx(now);
        self.server.poll(now);
        let mut a = self.air.borrow_mut();
        while let Some(f) = a.server_out.pop_front() {
            let to_modem = f[0] & 1 != 0 || f[..6] == a.mac;
            if a.link && to_modem {
                a.down.push_back(f);
            }
        }
    }

    fn run_until(&mut self, ms: i64, mut done: impl FnMut(&mut World) -> bool) -> bool {
        let end = self.now + ms;
        while self.now < end {
            self.step();
            if done(self) {
                return true;
            }
        }
        false
    }

    fn wwan(&self) -> Option<&ecm_net::Interface> {
        self.phone.stack.find_iface(IFNAME).and_then(|i| self.phone.stack.iface(i))
    }
}

fn udp_reaches(w: &mut World, port: u16) -> bool {
    let rx = w.server.stack.udp_bind(SocketAddr { ip: Ipv4Addr::ZERO, port }).unwrap();
    let tx = w.phone.stack.udp_bind(SocketAddr { ip: Ipv4Addr::ZERO, port: 0 }).unwrap();
    let now = w.now;
    let _ = w.phone.stack.udp_send_to(tx, SocketAddr { ip: Ipv4Addr([10, 9, 0, 1]), port }, b"hello", now);
    let ok = w.run_until(2_000, |w| {
        let mut buf = [0u8; 64];
        matches!(w.server.stack.udp_recv_from(rx, &mut buf), Ok(Some((_, 5))))
    });
    w.server.stack.udp_close(rx);
    w.phone.stack.udp_close(tx);
    ok
}

#[test]
fn wwan0_follows_the_modem_and_carries_traffic() {
    let mut w = World::new();
    w.run_until(2 * POLL_MS, |_| false);
    assert!(w.wwan().is_none(), "no modem, no wwan0");

    // A modem is fitted (no SIM yet: no carrier).
    w.air.borrow_mut().present = 1;
    assert!(w.run_until(2 * POLL_MS, |w| w.wwan().is_some()), "wwan0 appears with the module");
    assert_eq!(w.wwan().unwrap().mac.0, [0x02, 0xce, 0x11, 0, 0, 1], "wwan0 has the modem's MAC");
    assert!(!w.wwan().unwrap().is_up(), "no carrier before attach");
    let i = w.phone.stack.find_iface(IFNAME).unwrap();
    w.phone.stack.configure_addr(i, Ipv4Addr([10, 9, 0, 2]), 24, w.now);
    assert!(!udp_reaches(&mut w, 7000), "control: nothing gets through unattached");

    // Attached: carrier up, traffic flows both ways (ARP, then UDP).
    w.air.borrow_mut().link = true;
    assert!(w.run_until(2 * POLL_MS, |w| w.wwan().unwrap().is_up()), "carrier follows the modem");
    assert!(udp_reaches(&mut w, 7001), "UDP from the phone reaches the server over wwan0");

    // TCP: connect from the phone, then drop the carrier for 200 ms (a handover gap) and keep using it.
    let l = w.server.stack.tcp_listen(SocketAddr { ip: Ipv4Addr::ZERO, port: 22 }, 4).unwrap();
    let now = w.now;
    let c = w.phone.stack.tcp_connect(SocketAddr { ip: Ipv4Addr([10, 9, 0, 1]), port: 22 }, now).unwrap();
    assert!(w.run_until(3_000, |w| w.phone.stack.tcp_state(c) == Ok(TcpState::Established)), "TCP handshake over wwan0");
    let mut srv = None;
    assert!(w.run_until(1_000, |w| {
        srv = w.server.stack.tcp_accept(l).unwrap();
        srv.is_some()
    }));
    let srv = srv.unwrap();
    w.air.borrow_mut().link = false;
    w.run_until(200, |_| false);
    w.air.borrow_mut().link = true;
    let now = w.now;
    w.phone.stack.tcp_send(c, b"after the handover", now).unwrap();
    let mut got = Vec::new();
    assert!(w.run_until(5_000, |w| {
        let mut buf = [0u8; 64];
        if let Ok(n) = w.server.stack.tcp_recv(srv, &mut buf) {
            got.extend_from_slice(&buf[..n]);
        }
        got == b"after the handover"
    }), "the TCP connection survives a short carrier drop: got {:?}", String::from_utf8_lossy(&got));
    assert_eq!(w.phone.stack.tcp_state(c), Ok(TcpState::Established));

    // A different modem (new MAC): wwan0 is re-created with it.
    w.air.borrow_mut().mac = [0x02, 0xce, 0x22, 0, 0, 2];
    assert!(w.run_until(2 * POLL_MS, |w| w.wwan().is_some_and(|i| i.mac.0 == [0x02, 0xce, 0x22, 0, 0, 2])),
        "wwan0 takes the new modem's MAC");

    // Module removed: carrier down, wwan0 stays (its configuration comes back with a module).
    w.air.borrow_mut().present = 0;
    assert!(w.run_until(2 * POLL_MS, |w| !w.wwan().unwrap().is_up()), "carrier drops with the module");
}
