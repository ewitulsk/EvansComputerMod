//! `radiod` — IP over packet radio: a KISS-TNC style bridge that turns an SDR
//! into the `radio0` network interface, so `ping` and `ssh` run over slow
//! VHF packet (AFSK1200 over NBFM, AX.25 UI frames, like Linux `kissattach`).
//!
//!   radiod radio0 up sdr_0 144.39e6 --call N0CALL-1 --ip 10.44.0.1/24
//!          [--rate SPS] [--power DBM] [--txdelay MS] [--gain DB] [--seconds S] [-v]
//!
//! The interface exists while radiod runs (Ctrl+T or `kill` takes it down).
//! Every station on the channel needs its own callsign-SSID and address in
//! the same subnet. Data path (see ecm-radio `link::Tnc`):
//!
//!   kernel radio0 <-> tun socket <-> Ethernet<->AX.25 (+ARP, segmentation,
//!   MSS clamp) <-> KISS <-> AFSK1200/NBFM modem <-> /dev/sdr
//!
//! The radio is half duplex and deaf while transmitting, so channel access
//! matters: queued frames go out as one burst once the channel is clear.
//! Carrier sense uses the received power in absolute terms (sample power
//! minus the SDR's current AGC gain), against a tracked noise floor; after a
//! carrier or a decoded frame the station holds off 100-250 ms (jittered,
//! so two stations don't key up together).

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
    // AGC (stepped once per read; reads are ~20 ms so it settles within a
    // packet's TX delay).
    // With --gain the level is fixed, so a short --txdelay is enough.
    let mut sdr = setup_rx(&a.sdr, a.freq, a.rate, a.gain_db)?;
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
    let block = (a.rate as usize / 50).max(128);
    let total = a.seconds.map(|s| (s * rate) as u64);
    let mut done = 0u64;
    // Channel access: hold off while frames are arriving (a station's burst
    // can hold several), plus a little jitter so two stations don't key up
    // together after the same frame.
    let mut holdoff = 0u64;
    let mut carrier = ecm_radio::link::CarrierSense::default();
    let mut was_busy = false;
    let mut jitter = 0x9e37_79b9u32 ^ tnc.mac()[5] as u32;
    let mut buf = vec![0u8; 2048];
    let mut since_stats = 0u64;
    while total.map_or(true, |t| done < t) {
        // Receive.
        // Read everything that is waiting (up to the SDR's 0.25 s buffer), not a fixed slice: the
        // carrier-sense and transmit decisions below must see the channel as it is now. Reading
        // 100 ms per pass let a slow loop fall ~150 ms behind the air, so a station judged the
        // channel by stale samples and keyed up over a burst that had already started.
        let iq = sdr.read_at_least(block, (a.rate as usize / 4).max(block)).map_err(|e| format!("receive: {e}"))?;
        done += iq.len() as u64;
        since_stats += iq.len() as u64;
        if !iq.is_empty() {
            holdoff = holdoff.saturating_sub(iq.len() as u64);
            // Carrier sense on absolute power (dBFS - gain = dBm + const), see CarrierSense.
            let gain = sdr.status().ok().and_then(|st| st.num("gain")).unwrap_or(0.0) as f32;
            let mp = ecm_dsp::complex::mean_power(&iq);
            let busy = carrier.update(mp, gain);
            if a.verbose && busy != was_busy {
                println!("radiod: channel {} ({:.1} dBFS, floor {:.1}, gain {gain:.0}, at {})", if busy { "busy" } else { "clear" },
                    ecm_dsp::complex::to_db(mp), carrier.floor() + gain, sdr.timestamp().unwrap_or(0));
            }
            was_busy = busy;
            if busy {
                jitter = jitter.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                holdoff = holdoff.max((rate * 0.1) as u64 + (jitter >> 16) as u64 % (rate * 0.15) as u64);
            }
            let before = tnc.link().stats;
            let frames = tnc.from_air(&iq);
            if a.verbose && tnc.link().stats.rx_frames != before.rx_frames {
                println!("radiod: heard {} frame(s), {} packet(s) for us (at {})", tnc.link().stats.rx_frames - before.rx_frames, frames.len(), sdr.timestamp().unwrap_or(0));
            }
            for eth in frames {
                jitter = jitter.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                holdoff = holdoff.max((rate * 0.1) as u64 + (jitter >> 16) as u64 % (rate * 0.15) as u64);
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
            if a.verbose {
                let ty = if n >= 14 { u16::from_be_bytes([buf[12], buf[13]]) } else { 0 };
                println!("radiod: kernel sent {n} bytes (type {ty:04x}), {} frame(s) queued", tnc.pending());
            }
        }
        // Transmit when the channel is clear.
        if tnc.pending() > 0 && holdoff == 0 {
            let frames = tnc.pending();
            if let Some(burst) = tnc.take_burst() {
                if a.verbose {
                    println!("radiod: transmitting {frames} frame(s), {:.2} s (at {})", burst.len() as f64 / rate, sdr.timestamp().unwrap_or(0));
                }
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
