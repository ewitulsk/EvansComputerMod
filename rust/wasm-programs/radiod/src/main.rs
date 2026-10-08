//! `radiod` — IP over packet radio: a KISS-TNC style bridge that turns an SDR
//! into the `radio0` network interface, so `ping` and `ssh` run over slow
//! VHF packet (AFSK1200 over NBFM, AX.25 UI frames, like Linux `kissattach`).
//!
//!   radiod radio0 up sdr_0 144.39e6 --call N0CALL-1 --ip 10.44.0.1/24
//!          [--rate SPS] [--power DBM] [--txdelay MS] [--seconds S]
//!
//! The interface exists while radiod runs (Ctrl+T or `kill` takes it down).
//! Every station on the channel needs its own callsign-SSID and address in
//! the same subnet. Data path (see ecm-radio `link::Tnc`):
//!
//!   kernel radio0 <-> tun socket <-> Ethernet<->AX.25 (+ARP, segmentation,
//!   MSS clamp) <-> KISS <-> AFSK1200/NBFM modem <-> /dev/sdr
//!
//! The radio is half duplex: queued frames go out as one burst once no frame
//! has arrived for 100-250 ms (hold-off with jitter).

use ecm_dsp::coding::ax25::Address;
use ecm_host_abi::tun;
use ecm_radio::cli::{parse_radiod, RadiodArgs, RADIOD_USAGE};
use ecm_radio::device::TxPacer;
use ecm_radio::link::Tnc;
use ecm_radio::run::{args, fail, send_paced, setup_rx, wait_sent};
use ecm_radio::units::fmt_freq;

fn main() {
    let a = parse_radiod(&args()).unwrap_or_else(|e| fail("radiod", RADIOD_USAGE, e));
    if let Err(e) = run(&a) {
        eprintln!("radiod: {e}");
        std::process::exit(1);
    }
}

fn mac_str(m: [u8; 6]) -> String {
    m.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
}

fn run(a: &RadiodArgs) -> Result<(), String> {
    let me = Address::parse(&a.call).map_err(|_| format!("bad callsign {}", a.call))?;
    let rate = a.rate as f64;
    let mut tnc = Tnc::new(me, rate);
    tnc.set_txdelay_ms(a.txdelay_ms);
    // AGC (stepped once per read; reads are 10 ms so it settles within a
    // packet's TX delay).
    let mut sdr = setup_rx(&a.sdr, a.freq, a.rate, None)?;
    // Check the SDR can transmit before bringing the interface up.
    sdr.set_tx(true, a.power_dbm).map_err(|e| format!("{} can't transmit ({e}); use a Standard or Advanced SDR", a.sdr))?;
    sdr.set_tx(false, None).map_err(|e| e.to_string())?;
    let (ip, prefix) = a.ip.unwrap_or(([0; 4], 0));
    let fd = tun::open(&a.iface, tnc.mac(), ip, prefix);
    if fd < 0 {
        return Err(format!("can't create {} (already exists, or the kernel has no tun support): {fd}", a.iface));
    }
    println!(
        "radiod: {} up on {} {} as {} (mac {}{}), AFSK1200/NBFM  [Ctrl+T stops]",
        a.iface,
        a.sdr,
        fmt_freq(a.freq),
        a.call,
        mac_str(tnc.mac()),
        a.ip.map(|(ip, p)| format!(", {}.{}.{}.{}/{p}", ip[0], ip[1], ip[2], ip[3])).unwrap_or_default()
    );
    let result = serve(a, &mut tnc, &mut sdr, fd);
    ecm_host_abi::socket::close(fd);
    let s = tnc.link().stats;
    println!(
        "radiod: {} down; sent {} packets in {} frames, received {} packets from {} frames, dropped {}",
        a.iface, s.tx_packets, s.tx_frames, s.rx_packets, s.rx_frames, s.dropped
    );
    result
}

fn serve(a: &RadiodArgs, tnc: &mut Tnc, sdr: &mut ecm_radio::device::Sdr, fd: i32) -> Result<(), String> {
    let rate = a.rate as f64;
    let block = (a.rate as usize / 100).max(128);
    let total = a.seconds.map(|s| (s * rate) as u64);
    let mut done = 0u64;
    // Channel access: hold off while frames are arriving (a station's burst
    // can hold several), plus a little jitter so two stations don't key up
    // together after the same frame.
    let mut holdoff = 0u64;
    let mut jitter = 0x9e37_79b9u32 ^ tnc.mac()[5] as u32;
    let mut buf = vec![0u8; 2048];
    let mut since_stats = 0u64;
    while total.map_or(true, |t| done < t) {
        // Receive.
        let iq = sdr.read(block).map_err(|e| format!("receive: {e}"))?;
        done += iq.len() as u64;
        since_stats += iq.len() as u64;
        if !iq.is_empty() {
            holdoff = holdoff.saturating_sub(iq.len() as u64);
            for eth in tnc.from_air(&iq) {
                jitter = jitter.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                holdoff = (rate * 0.1) as u64 + (jitter >> 16) as u64 % (rate * 0.15) as u64;
                if tun::write(fd, &eth) < 0 {
                    return Err(format!("{} went away", a.iface));
                }
            }
        }
        // Frames from the kernel.
        loop {
            let n = tun::read(fd, &mut buf);
            if n == tun::TUN_EMPTY {
                break;
            }
            if n < 0 {
                return Err(format!("{} went away ({n})", a.iface));
            }
            tnc.from_kernel(&buf[..n as usize]);
        }
        // Transmit when the channel is clear.
        if tnc.pending() > 0 && holdoff == 0 {
            if let Some(burst) = tnc.take_burst() {
                sdr.set_tx(true, a.power_dbm).map_err(|e| format!("tx on: {e}"))?;
                let mut pacer = TxPacer::new(rate, 0.3);
                let r = send_paced(sdr, &mut pacer, &burst).and_then(|_| wait_sent(sdr, &pacer));
                sdr.set_tx(false, None).map_err(|e| format!("tx off: {e}"))?;
                r?;
                done += burst.len() as u64;
            }
        }
        if since_stats >= rate as u64 * 60 {
            since_stats = 0;
            let s = tnc.link().stats;
            println!("radiod: tx {} pkts / rx {} pkts, {} stations heard", s.tx_packets, s.rx_packets, tnc.link().peers().count());
        }
    }
    Ok(())
}
