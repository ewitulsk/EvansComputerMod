//! TCP connection state machine (one TCB). Sans-IO: every method appends the
//! segments it wants sent to `out`; the stack wraps them in IP and routes them.

use std::collections::VecDeque;

use crate::tcp::*;
use crate::types::{NetError, SocketAddr};

/// Receive buffer (also the max advertised window; no window scaling).
pub const RX_CAP: usize = 32 * 1024;
/// Send buffer (unacked + unsent bytes).
pub const TX_CAP: usize = 64 * 1024;
pub const INITIAL_RTO_MS: i64 = 1000;
pub const MIN_RTO_MS: i64 = 200;
pub const MAX_RTO_MS: i64 = 60_000;
/// Retransmissions of the same data before the connection is aborted.
pub const MAX_RETRIES: u8 = 10;
pub const MSL_MS: i64 = 15_000;
pub const TIME_WAIT_MS: i64 = 2 * MSL_MS;
pub const FIN_WAIT2_TIMEOUT_MS: i64 = 60_000;
/// Out-of-order segments held for reassembly (drop newest when full).
const MAX_OOO: usize = 32;

/// A segment to transmit (addresses/ports come from the TCB).
#[derive(Debug, Clone)]
pub struct Seg {
    pub seq: u32,
    pub ack: u32,
    pub flags: u8,
    pub window: u16,
    pub mss: Option<u16>,
    pub payload: Vec<u8>,
}

pub struct Tcb {
    pub state: TcpState,
    pub local: SocketAddr,
    pub remote: SocketAddr,

    iss: u32,
    snd_una: u32,
    snd_nxt: u32,
    /// Highest sequence number ever sent (+1). `snd_nxt` may be rewound below it.
    snd_max: u32,
    snd_wnd: u32,
    snd_wl1: u32,
    snd_wl2: u32,
    snd_mss: u16,
    tx: VecDeque<u8>,
    /// User asked for FIN (shutdown/close); sent once `tx` drains.
    fin_pending: bool,
    /// Sequence number of our FIN once it has been sent.
    fin_seq: Option<u32>,
    fin_acked: bool,

    rcv_nxt: u32,
    rx: VecDeque<u8>,
    ooo: Vec<(u32, Vec<u8>)>,
    /// Sequence number of the peer's FIN, once seen in-window.
    rx_fin_seq: Option<u32>,
    pub fin_received: bool,
    last_adv_wnd: u32,
    /// User closed the handle: incoming data is ACKed and discarded.
    discard_rx: bool,

    rto: i64,
    srtt: Option<i64>,
    rttvar: i64,
    rtt_probe: Option<(u32, i64)>,
    rto_deadline: Option<i64>,
    retries: u8,
    dupacks: u8,
    close_deadline: Option<i64>,

    pub error: Option<NetError>,
    /// The user holds a handle to this TCB.
    pub user_open: bool,
    pub retransmits: u64,
}

impl Tcb {
    fn blank(local: SocketAddr, remote: SocketAddr, iss: u32, state: TcpState) -> Self {
        Tcb {
            state,
            local,
            remote,
            iss,
            snd_una: iss,
            snd_nxt: iss,
            snd_max: iss,
            snd_wnd: 0,
            snd_wl1: 0,
            snd_wl2: 0,
            snd_mss: DEFAULT_MSS,
            tx: VecDeque::new(),
            fin_pending: false,
            fin_seq: None,
            fin_acked: false,
            rcv_nxt: 0,
            rx: VecDeque::new(),
            ooo: Vec::new(),
            rx_fin_seq: None,
            fin_received: false,
            last_adv_wnd: 0,
            discard_rx: false,
            rto: INITIAL_RTO_MS,
            srtt: None,
            rttvar: 0,
            rtt_probe: None,
            rto_deadline: None,
            retries: 0,
            dupacks: 0,
            close_deadline: None,
            error: None,
            user_open: true,
            retransmits: 0,
        }
    }

    /// A listening socket (never sends; SYNs are handled by the stack).
    pub fn new_listen(local: SocketAddr) -> Self {
        Self::blank(local, SocketAddr::default(), 0, TcpState::Listen)
    }

    /// Active open: emits a SYN.
    pub fn new_connect(local: SocketAddr, remote: SocketAddr, iss: u32, now: i64, out: &mut Vec<Seg>) -> Self {
        let mut t = Self::blank(local, remote, iss, TcpState::SynSent);
        t.snd_nxt = iss.wrapping_add(1);
        t.snd_max = t.snd_nxt;
        t.send_syn(out);
        t.rto_deadline = Some(now + t.rto);
        t
    }

    /// Passive open from a received SYN: emits a SYN-ACK. The child is not user-owned
    /// until accepted.
    pub fn new_passive(local: SocketAddr, remote: SocketAddr, iss: u32, syn: &TcpHeader, now: i64, out: &mut Vec<Seg>) -> Self {
        let mut t = Self::blank(local, remote, iss, TcpState::SynReceived);
        t.user_open = false;
        t.rcv_nxt = syn.seq_num.wrapping_add(1);
        t.snd_nxt = iss.wrapping_add(1);
        t.snd_max = t.snd_nxt;
        t.snd_wnd = syn.window as u32;
        t.snd_wl1 = syn.seq_num;
        t.snd_mss = clamp_mss(syn.mss);
        t.send_syn(out);
        t.rto_deadline = Some(now + t.rto);
        t
    }

    // ------------------------------------------------------------------ helpers

    fn rcv_wnd(&self) -> u32 {
        if self.discard_rx {
            RX_CAP as u32
        } else {
            (RX_CAP - self.rx.len().min(RX_CAP)) as u32
        }
    }

    fn adv_window(&mut self) -> u16 {
        let w = self.rcv_wnd().min(u16::MAX as u32);
        self.last_adv_wnd = w;
        w as u16
    }

    fn seg(&mut self, seq: u32, flags: u8, payload: Vec<u8>) -> Seg {
        let window = self.adv_window();
        Seg { seq, ack: self.rcv_nxt, flags, window, mss: None, payload }
    }

    fn send_syn(&mut self, out: &mut Vec<Seg>) {
        let flags = if self.state == TcpState::SynSent { SYN } else { SYN | ACK };
        let mut s = self.seg(self.iss, flags, Vec::new());
        if flags & ACK == 0 {
            s.ack = 0;
        }
        s.mss = Some(TCP_MSS);
        out.push(s);
    }

    fn send_ack(&mut self, out: &mut Vec<Seg>) {
        let s = self.seg(self.snd_nxt, ACK, Vec::new());
        out.push(s);
    }

    fn synchronized(&self) -> bool {
        !matches!(self.state, TcpState::Closed | TcpState::Listen | TcpState::SynSent | TcpState::SynReceived)
    }

    fn can_send_data(&self) -> bool {
        matches!(
            self.state,
            TcpState::Established | TcpState::CloseWait | TcpState::FinWait1 | TcpState::Closing | TcpState::LastAck
        )
    }

    fn set_closed(&mut self, err: Option<NetError>) {
        self.state = TcpState::Closed;
        if self.error.is_none() {
            self.error = err;
        }
        self.rto_deadline = None;
        self.close_deadline = None;
        self.tx.clear();
        self.ooo.clear();
    }

    fn enter_time_wait(&mut self, now: i64) {
        self.state = TcpState::TimeWait;
        self.rto_deadline = None;
        self.close_deadline = Some(now + TIME_WAIT_MS);
        self.tx.clear();
    }

    fn enter_fin_wait2(&mut self, now: i64) {
        self.state = TcpState::FinWait2;
        self.rto_deadline = None;
        if !self.user_open {
            self.close_deadline = Some(now + FIN_WAIT2_TIMEOUT_MS);
        }
    }

    fn update_rtt(&mut self, sample: i64) {
        let r = sample.max(1);
        match self.srtt {
            None => {
                self.srtt = Some(r);
                self.rttvar = r / 2;
            }
            Some(s) => {
                self.rttvar = (3 * self.rttvar + (s - r).abs()) / 4;
                self.srtt = Some((7 * s + r) / 8);
            }
        }
        let s = self.srtt.unwrap_or(r);
        self.rto = (s + (4 * self.rttvar).max(10)).clamp(MIN_RTO_MS, MAX_RTO_MS);
    }

    /// Bytes of data (not SYN/FIN) sent but unacknowledged, measured to `snd_nxt`.
    fn data_in_flight(&self) -> usize {
        (self.snd_nxt.wrapping_sub(self.snd_una) as usize).min(self.tx.len())
    }

    // --------------------------------------------------------------- user side

    pub fn rx_available(&self) -> usize {
        self.rx.len()
    }

    pub fn tx_space(&self) -> usize {
        TX_CAP.saturating_sub(self.tx.len())
    }

    pub fn tx_pending(&self) -> usize {
        self.tx.len()
    }

    /// Queue data; returns bytes accepted (0 if full).
    pub fn write(&mut self, data: &[u8], now: i64, out: &mut Vec<Seg>) -> usize {
        let n = data.len().min(self.tx_space());
        self.tx.extend(&data[..n]);
        self.output(now, out, false, false);
        n
    }

    /// Read buffered data; may emit a window update.
    pub fn read(&mut self, buf: &mut [u8], out: &mut Vec<Seg>) -> usize {
        let n = buf.len().min(self.rx.len());
        for (d, s) in buf.iter_mut().zip(self.rx.drain(..n)) {
            *d = s;
        }
        if n > 0 && self.synchronized() && !self.fin_received {
            let w = self.rcv_wnd();
            let step = (2 * TCP_MSS as u32).min(RX_CAP as u32 / 2);
            if w >= self.last_adv_wnd.saturating_add(step) || (self.last_adv_wnd == 0 && w > 0) {
                self.send_ack(out);
            }
        }
        n
    }

    /// Half-close: FIN once TX drains.
    pub fn shutdown_write(&mut self, now: i64, out: &mut Vec<Seg>) {
        match self.state {
            TcpState::Established => {
                self.fin_pending = true;
                self.state = TcpState::FinWait1;
                self.output(now, out, false, false);
            }
            TcpState::CloseWait => {
                self.fin_pending = true;
                self.state = TcpState::LastAck;
                self.output(now, out, false, false);
            }
            _ => {}
        }
    }

    /// User close (graceful). Returns true if the TCB should be aborted instead
    /// (states with no established connection).
    pub fn close(&mut self, now: i64, out: &mut Vec<Seg>) -> bool {
        self.user_open = false;
        self.discard_rx = true;
        self.rx.clear();
        match self.state {
            TcpState::Listen | TcpState::SynSent | TcpState::SynReceived | TcpState::Closed => return true,
            TcpState::Established | TcpState::CloseWait => self.shutdown_write(now, out),
            TcpState::FinWait2 => self.close_deadline = Some(now + FIN_WAIT2_TIMEOUT_MS),
            _ => {}
        }
        false
    }

    /// Abort: RST (if synchronized) and close.
    pub fn abort(&mut self, out: &mut Vec<Seg>) {
        if self.synchronized() || self.state == TcpState::SynReceived {
            out.push(Seg { seq: self.snd_nxt, ack: 0, flags: RST, window: 0, mss: None, payload: Vec::new() });
        }
        self.set_closed(Some(NetError::ConnectionAborted));
    }

    /// Local teardown (interface removed, ...): Closed with `err`, nothing sent.
    pub fn fail(&mut self, err: NetError) {
        self.set_closed(Some(err));
    }

    pub fn rcv_nxt(&self) -> u32 {
        self.rcv_nxt
    }

    /// ARP / routing failure for our packets.
    pub fn on_unreachable(&mut self) {
        if matches!(self.state, TcpState::SynSent) {
            self.set_closed(Some(NetError::HostUnreachable));
        }
    }

    // ------------------------------------------------------------------ output

    /// Send whatever the window allows (data, then FIN). `force_ack` emits a bare ACK
    /// if nothing else was sent. `probe` allows one byte past a zero window.
    fn output(&mut self, now: i64, out: &mut Vec<Seg>, force_ack: bool, probe: bool) {
        let mut sent_any = false;
        let mut probe = probe;
        if self.can_send_data() {
            loop {
                let offset = self.snd_nxt.wrapping_sub(self.snd_una) as usize;
                if offset > self.tx.len() {
                    break; // FIN in flight
                }
                let unsent = self.tx.len() - offset;
                let wnd_end = self.snd_una.wrapping_add(self.snd_wnd);
                let mut usable = if seq_lt(self.snd_nxt, wnd_end) { wnd_end.wrapping_sub(self.snd_nxt) as usize } else { 0 };
                if probe && usable == 0 && unsent > 0 {
                    usable = 1;
                }
                probe = false;
                let len = unsent.min(usable).min(self.snd_mss as usize);
                let fin_now = self.fin_pending && self.fin_seq.map_or(true, |f| seq_ge(f, self.snd_nxt)) && offset + len == self.tx.len();
                if len == 0 && !fin_now {
                    break;
                }
                let payload: Vec<u8> = self.tx.range(offset..offset + len).copied().collect();
                let mut flags = ACK;
                if len > 0 {
                    flags |= PSH;
                }
                if fin_now {
                    flags |= FIN;
                }
                let seq = self.snd_nxt;
                let s = self.seg(seq, flags, payload);
                out.push(s);
                let is_new = seq_ge(seq, self.snd_max);
                if !is_new {
                    self.retransmits += 1;
                }
                self.snd_nxt = seq.wrapping_add(len as u32 + fin_now as u32);
                if seq_gt(self.snd_nxt, self.snd_max) {
                    self.snd_max = self.snd_nxt;
                }
                if fin_now {
                    self.fin_seq = Some(self.snd_nxt.wrapping_sub(1));
                }
                if is_new && self.rtt_probe.is_none() && len > 0 {
                    self.rtt_probe = Some((self.snd_nxt, now));
                }
                sent_any = true;
                if fin_now {
                    break;
                }
            }
            // Timer: armed while anything is unacknowledged, or as a persist timer
            // when the peer's window is closed and we have data.
            let outstanding = self.snd_una != self.snd_max;
            let persist = self.snd_wnd == 0 && self.tx.len() > self.data_in_flight();
            if (outstanding || persist) && self.rto_deadline.is_none() {
                self.rto_deadline = Some(now + self.rto);
            }
        }
        if !sent_any && force_ack {
            self.send_ack(out);
        }
    }

    /// Retransmit one segment starting at snd_una (fast retransmit).
    fn retransmit_first(&mut self, out: &mut Vec<Seg>) {
        let len = self.tx.len().min(self.snd_mss as usize);
        let mut flags = ACK;
        if len > 0 {
            flags |= PSH;
        }
        if let Some(f) = self.fin_seq {
            if f == self.snd_una.wrapping_add(len as u32) {
                flags |= FIN;
            }
        }
        if len == 0 && flags & FIN == 0 {
            return;
        }
        let payload: Vec<u8> = self.tx.range(..len).copied().collect();
        let s = self.seg(self.snd_una, flags, payload);
        out.push(s);
        self.retransmits += 1;
        self.rtt_probe = None;
    }

    // ------------------------------------------------------------------ timers

    pub fn next_deadline(&self) -> Option<i64> {
        match (self.rto_deadline, self.close_deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    pub fn on_timer(&mut self, now: i64, out: &mut Vec<Seg>) {
        if let Some(d) = self.close_deadline {
            if now >= d && matches!(self.state, TcpState::TimeWait | TcpState::FinWait2) {
                self.set_closed(None);
                return;
            }
        }
        let Some(d) = self.rto_deadline else { return };
        if now < d {
            return;
        }
        self.rto_deadline = None;
        let persist = self.snd_wnd == 0 && self.snd_una == self.snd_max && !self.tx.is_empty();
        if !persist {
            self.retries = self.retries.saturating_add(1);
            if self.retries > MAX_RETRIES {
                self.set_closed(Some(NetError::TimedOut));
                return;
            }
        }
        self.rto = (self.rto * 2).min(MAX_RTO_MS);
        self.rtt_probe = None;
        self.dupacks = 0;
        match self.state {
            TcpState::SynSent | TcpState::SynReceived => {
                self.send_syn(out);
                self.retransmits += 1;
            }
            _ if self.can_send_data() => {
                self.snd_nxt = self.snd_una;
                self.output(now, out, false, true);
            }
            _ => {}
        }
        if self.rto_deadline.is_none() && self.state != TcpState::Closed && (self.snd_una != self.snd_max || persist) {
            self.rto_deadline = Some(now + self.rto);
        }
    }

    // --------------------------------------------------------------- input

    /// Process one segment for this connection (state != Listen).
    pub fn on_segment(&mut self, h: &TcpHeader, payload: &[u8], now: i64, out: &mut Vec<Seg>) {
        match self.state {
            TcpState::Closed | TcpState::Listen => {}
            TcpState::SynSent => self.on_syn_sent(h, payload, now, out),
            _ => self.on_synchronized(h, payload, now, out),
        }
    }

    fn on_syn_sent(&mut self, h: &TcpHeader, payload: &[u8], now: i64, out: &mut Vec<Seg>) {
        let ack_ok = h.has(ACK) && seq_gt(h.ack_num, self.iss) && seq_le(h.ack_num, self.snd_max);
        if h.has(ACK) && !ack_ok {
            if !h.has(RST) {
                out.push(Seg { seq: h.ack_num, ack: 0, flags: RST, window: 0, mss: None, payload: Vec::new() });
            }
            return;
        }
        if h.has(RST) {
            if ack_ok {
                self.set_closed(Some(NetError::ConnectionRefused));
            }
            return;
        }
        if !h.has(SYN) {
            return;
        }
        self.rcv_nxt = h.seq_num.wrapping_add(1);
        self.snd_mss = clamp_mss(h.mss);
        if ack_ok {
            self.snd_una = h.ack_num;
            self.snd_wnd = h.window as u32;
            self.snd_wl1 = h.seq_num;
            self.snd_wl2 = h.ack_num;
            self.state = TcpState::Established;
            self.rto_deadline = None;
            self.retries = 0;
            self.rto = INITIAL_RTO_MS;
            let _ = payload; // data on SYN-ACK is not accepted (peer will retransmit)
            self.output(now, out, true, false);
        } else {
            // simultaneous open
            self.state = TcpState::SynReceived;
            self.snd_wnd = h.window as u32;
            self.snd_wl1 = h.seq_num;
            self.send_syn(out);
            self.rto_deadline = Some(now + self.rto);
        }
    }

    fn acceptable(&self, seq: u32, seg_len: u32) -> bool {
        let wnd = self.rcv_wnd();
        let in_wnd = |s: u32| seq_ge(s, self.rcv_nxt) && seq_lt(s, self.rcv_nxt.wrapping_add(wnd));
        match (seg_len, wnd) {
            (0, 0) => seq == self.rcv_nxt,
            (0, _) => in_wnd(seq),
            (_, 0) => false,
            _ => in_wnd(seq) || in_wnd(seq.wrapping_add(seg_len - 1)),
        }
    }

    fn on_synchronized(&mut self, h: &TcpHeader, payload: &[u8], now: i64, out: &mut Vec<Seg>) {
        let seq = h.seq_num;
        let seg_len = payload.len() as u32 + h.has(SYN) as u32 + h.has(FIN) as u32;

        // Retransmitted SYN while we wait for the handshake ACK: resend SYN-ACK.
        if self.state == TcpState::SynReceived && h.has(SYN) && !h.has(ACK) && seq.wrapping_add(1) == self.rcv_nxt {
            self.send_syn(out);
            return;
        }

        // 1. sequence check (FIN-only at rcv_nxt is accepted even with a zero window)
        let fin_only_in_order = payload.is_empty() && h.has(FIN) && !h.has(SYN) && seq == self.rcv_nxt;
        if !self.acceptable(seq, seg_len) && !fin_only_in_order {
            if !h.has(RST) {
                if self.state == TcpState::TimeWait && h.has(FIN) {
                    self.close_deadline = Some(now + TIME_WAIT_MS);
                }
                self.send_ack(out);
            }
            return;
        }

        // 2. RST (RFC 5961: exact match resets, in-window gets a challenge ACK)
        if h.has(RST) {
            if seq != self.rcv_nxt {
                self.send_ack(out);
                return;
            }
            let err = match self.state {
                TcpState::SynReceived if self.user_open => Some(NetError::ConnectionRefused),
                TcpState::SynReceived | TcpState::Closing | TcpState::LastAck | TcpState::TimeWait => None,
                _ => Some(NetError::ConnectionReset),
            };
            self.set_closed(err);
            return;
        }

        // 3. SYN in a synchronized state: challenge ACK
        if h.has(SYN) {
            self.send_ack(out);
            return;
        }

        // 4. ACK
        if !h.has(ACK) {
            return;
        }
        let ack = h.ack_num;
        if self.state == TcpState::SynReceived {
            if seq_gt(ack, self.snd_una) && seq_le(ack, self.snd_max) {
                self.state = TcpState::Established;
                self.snd_una = ack;
                self.snd_wnd = h.window as u32;
                self.snd_wl1 = seq;
                self.snd_wl2 = ack;
                self.rto_deadline = None;
                self.retries = 0;
                self.rto = INITIAL_RTO_MS;
            } else {
                out.push(Seg { seq: ack, ack: 0, flags: RST, window: 0, mss: None, payload: Vec::new() });
                return;
            }
        } else {
            if seq_gt(ack, self.snd_max) {
                self.send_ack(out);
                return;
            }
            if seq_gt(ack, self.snd_una) {
                let n = ack.wrapping_sub(self.snd_una) as usize;
                if let Some(f) = self.fin_seq {
                    if ack == f.wrapping_add(1) {
                        self.fin_acked = true;
                    }
                }
                let drain = n.min(self.tx.len());
                self.tx.drain(..drain);
                self.snd_una = ack;
                if seq_lt(self.snd_nxt, ack) {
                    self.snd_nxt = ack;
                }
                if let Some((pseq, t)) = self.rtt_probe {
                    if seq_ge(ack, pseq) {
                        self.update_rtt(now - t);
                        self.rtt_probe = None;
                    }
                }
                self.retries = 0;
                self.dupacks = 0;
                self.rto_deadline = if self.snd_una == self.snd_max { None } else { Some(now + self.rto) };
            } else if ack == self.snd_una {
                if payload.is_empty() && !h.has(FIN) && h.window as u32 == self.snd_wnd && self.snd_una != self.snd_max {
                    self.dupacks = self.dupacks.saturating_add(1);
                    if self.dupacks == 3 {
                        self.retransmit_first(out);
                    }
                }
                if h.window == 0 {
                    self.retries = 0; // peer is alive, just not reading
                }
            }
            if seq_lt(self.snd_wl1, seq) || (self.snd_wl1 == seq && seq_le(self.snd_wl2, ack)) {
                self.snd_wnd = h.window as u32;
                self.snd_wl1 = seq;
                self.snd_wl2 = ack;
            }
        }

        // FIN acknowledged?
        if self.fin_acked {
            match self.state {
                TcpState::FinWait1 => self.enter_fin_wait2(now),
                TcpState::Closing => self.enter_time_wait(now),
                TcpState::LastAck => {
                    self.set_closed(None);
                    return;
                }
                _ => {}
            }
        }

        // 5. data
        let mut need_ack = false;
        if matches!(self.state, TcpState::Established | TcpState::FinWait1 | TcpState::FinWait2) {
            if !payload.is_empty() {
                self.accept_data(seq, payload);
                need_ack = true;
            }
            if h.has(FIN) {
                let f = seq.wrapping_add(payload.len() as u32);
                if seq_ge(f, self.rcv_nxt) && self.rx_fin_seq.is_none() {
                    self.rx_fin_seq = Some(f);
                }
                need_ack = true;
            }
        } else if h.has(FIN) || !payload.is_empty() {
            need_ack = true; // retransmitted FIN/data after the peer's FIN: re-ACK
        }

        // 6. FIN (only once all data before it has arrived)
        if !self.fin_received && self.rx_fin_seq == Some(self.rcv_nxt) {
            self.fin_received = true;
            self.rcv_nxt = self.rcv_nxt.wrapping_add(1);
            self.ooo.clear();
            match self.state {
                TcpState::Established => self.state = TcpState::CloseWait,
                TcpState::FinWait1 => {
                    if self.fin_acked {
                        self.enter_time_wait(now)
                    } else {
                        self.state = TcpState::Closing
                    }
                }
                TcpState::FinWait2 => self.enter_time_wait(now),
                _ => {}
            }
        }
        if self.state == TcpState::TimeWait && h.has(FIN) {
            self.close_deadline = Some(now + TIME_WAIT_MS);
        }

        self.output(now, out, need_ack, false);
    }

    fn accept_data(&mut self, seq: u32, payload: &[u8]) {
        let (mut seq, mut data) = (seq, payload);
        if seq_lt(seq, self.rcv_nxt) {
            let skip = self.rcv_nxt.wrapping_sub(seq) as usize;
            if skip >= data.len() {
                return;
            }
            data = &data[skip..];
            seq = self.rcv_nxt;
        }
        let wnd = self.rcv_wnd() as usize;
        let off = seq.wrapping_sub(self.rcv_nxt) as usize;
        if off >= wnd {
            return;
        }
        let max_len = wnd - off;
        if data.len() > max_len {
            data = &data[..max_len];
        }
        if data.is_empty() {
            return;
        }
        if seq == self.rcv_nxt {
            self.push_rx(data);
            self.drain_ooo();
        } else if self.ooo.len() < MAX_OOO && !self.ooo.iter().any(|(s, d)| *s == seq && d.len() >= data.len()) {
            self.ooo.push((seq, data.to_vec()));
        }
    }

    fn push_rx(&mut self, data: &[u8]) {
        if !self.discard_rx {
            self.rx.extend(data);
        }
        self.rcv_nxt = self.rcv_nxt.wrapping_add(data.len() as u32);
    }

    fn drain_ooo(&mut self) {
        loop {
            let Some(i) = self.ooo.iter().position(|(s, _)| seq_le(*s, self.rcv_nxt)) else { break };
            let (s, d) = self.ooo.swap_remove(i);
            let skip = self.rcv_nxt.wrapping_sub(s) as usize;
            if skip < d.len() {
                self.push_rx(&d[skip..]);
            }
        }
    }
}

fn clamp_mss(mss: Option<u16>) -> u16 {
    mss.unwrap_or(DEFAULT_MSS).clamp(64, TCP_MSS)
}
