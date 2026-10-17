//! `_radio` native module: the per-sample work behind the Lua `radio` module
//! (lua/radio.lua), on top of `ecm-radio` / `ecm-dsp`, with the same functions
//! as the Python `_radio`. Samples cross into Lua as packed strings
//! (little-endian f32: interleaved I/Q for complex streams, one value per
//! sample for real streams) and frames as lists of strings; Lua only wires
//! blocks.
//!
//! Chains are kept in a per-thread table and addressed by integer handles.

use crate::values::bytes;
use ecm_dsp::coding::ax25::{Address, UiFrame};
use ecm_dsp::iq::SampleFormat;
use ecm_radio::audio;
use ecm_radio::blocks::{Buf, Chain, Kind, Param, Spec};
use ecm_radio::device::{resolve, Status};
use ecm_radio::sigmf::{format_from_datatype, Meta};
use ecm_radio::C32;
use mlua::{Lua, MultiValue, Table, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;

thread_local! {
    static CHAINS: RefCell<(i64, BTreeMap<i64, Chain>)> = RefCell::new((0, BTreeMap::new()));
}

fn err(msg: impl Into<String>) -> mlua::Error {
    mlua::Error::runtime(msg.into())
}

fn c32_to_bytes(x: &[C32]) -> Vec<u8> {
    let mut v = Vec::with_capacity(x.len() * 8);
    for s in x {
        v.extend_from_slice(&s.re.to_le_bytes());
        v.extend_from_slice(&s.im.to_le_bytes());
    }
    v
}

fn bytes_to_c32(b: &[u8]) -> Vec<C32> {
    b.chunks_exact(8)
        .map(|c| C32::new(f32::from_le_bytes([c[0], c[1], c[2], c[3]]), f32::from_le_bytes([c[4], c[5], c[6], c[7]])))
        .collect()
}

fn f32_to_bytes(x: &[f32]) -> Vec<u8> {
    x.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn bytes_to_f32(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn kind_of(s: &str) -> mlua::Result<Kind> {
    Kind::parse(s).ok_or_else(|| err(format!("unknown stream kind {s:?} (complex, real, frames)")))
}

fn to_buf(kind: Kind, data: &Value) -> mlua::Result<Buf> {
    Ok(match kind {
        Kind::Complex => Buf::C(bytes_to_c32(&bytes(data, "complex samples")?)),
        Kind::Real => Buf::R(bytes_to_f32(&bytes(data, "real samples")?)),
        Kind::Frames => {
            let Value::Table(t) = data else {
                return Err(err("frames must be a list of strings"));
            };
            let mut out = Vec::new();
            for v in t.clone().sequence_values::<Value>() {
                out.push(bytes(&v?, "frame")?);
            }
            Buf::F(out)
        }
    })
}

fn from_buf(lua: &Lua, b: Buf) -> mlua::Result<Value> {
    Ok(match b {
        Buf::C(x) => Value::String(lua.create_string(c32_to_bytes(&x))?),
        Buf::R(x) => Value::String(lua.create_string(f32_to_bytes(&x))?),
        Buf::F(f) => {
            let t = lua.create_table_with_capacity(f.len(), 0)?;
            for (i, v) in f.into_iter().enumerate() {
                t.raw_set(i + 1, lua.create_string(v)?)?;
            }
            Value::Table(t)
        }
    })
}

fn param_of(v: &Value) -> mlua::Result<Param> {
    Ok(match v {
        Value::String(s) => Param::Str(s.to_str()?.to_string()),
        Value::Boolean(b) => Param::Num(if *b { 1.0 } else { 0.0 }),
        Value::Integer(i) => Param::Num(*i as f64),
        Value::Number(n) => Param::Num(*n),
        other => return Err(err(format!("block parameters must be numbers or strings, got a {}", other.type_name()))),
    })
}

fn sample_format(name: &str) -> mlua::Result<SampleFormat> {
    format_from_datatype(name).ok_or_else(|| err(format!("unknown sample format {name:?}")))
}

fn str_of(lua: &Lua, s: impl AsRef<[u8]>) -> mlua::Result<Value> {
    Ok(Value::String(lua.create_string(s)?))
}

pub fn module(lua: &Lua) -> mlua::Result<Table> {
    let m = lua.create_table()?;

    // Build a chain: blocks = { {name, {param = value}}, ... }; returns
    // handle, out_kind, out_rate, description.
    m.set("chain_new", lua.create_function(|_, (blocks, kind, rate): (Table, String, f64)| {
        let kind = kind_of(&kind)?;
        let mut specs = Vec::new();
        for b in blocks.sequence_values::<Table>() {
            let b = b?;
            let name: String = b.get(1)?;
            let mut p = BTreeMap::new();
            if let Ok(params) = b.get::<Table>(2) {
                for pair in params.pairs::<String, Value>() {
                    let (k, v) = pair?;
                    p.insert(k, param_of(&v)?);
                }
            }
            specs.push(Spec::from_params(&name, &p).map_err(err)?);
        }
        let chain = Chain::build(specs, kind, rate).map_err(err)?;
        let (ok, orate, desc) = (chain.out_kind(), chain.out_rate(), chain.describe());
        let h = CHAINS.with(|c| {
            let mut c = c.borrow_mut();
            c.0 += 1;
            let h = c.0;
            c.1.insert(h, chain);
            h
        });
        Ok((h, ok.name().to_string(), orate, desc))
    })?)?;

    // Run one buffer through a chain; returns its output (a string or a list of strings).
    m.set("chain_process", lua.create_function(|lua, (handle, kind, data): (i64, String, Value)| {
        let buf = to_buf(kind_of(&kind)?, &data)?;
        let out = CHAINS.with(|c| match c.borrow_mut().1.get_mut(&handle) {
            Some(ch) => ch.process(buf),
            None => Err(format!("no chain {handle}")),
        });
        from_buf(lua, out.map_err(err)?)
    })?)?;

    m.set("chain_free", lua.create_function(|_, handle: i64| {
        CHAINS.with(|c| c.borrow_mut().1.remove(&handle));
        Ok(())
    })?)?;

    // Device / file bytes in `fmt` (cs16, cf32, ...) -> packed complex, and back.
    m.set("decode", lua.create_function(|lua, (fmt, data): (String, Value)| {
        let mut out = Vec::new();
        sample_format(&fmt)?.decode(&bytes(&data, "data")?, &mut out);
        str_of(lua, c32_to_bytes(&out))
    })?)?;
    m.set("encode", lua.create_function(|lua, (fmt, data): (String, Value)| {
        let mut out = Vec::new();
        sample_format(&fmt)?.encode(&bytes_to_c32(&bytes(&data, "data")?), &mut out);
        str_of(lua, out)
    })?)?;
    m.set("sample_size", lua.create_function(|_, fmt: String| Ok(sample_format(&fmt)?.bytes_per_sample()))?)?;

    // n samples of a tone at `hz` starting at `phase` (radians); returns
    // data, next_phase. complex = e^{j w t}, real = cos.
    m.set("tone", lua.create_function(
        |lua, (hz, rate, n, amp, mut ph, kind): (f64, f64, usize, f64, f64, String)| {
            if rate <= 0.0 {
                return Err(err("rate must be > 0"));
            }
            let w = std::f64::consts::TAU * hz / rate;
            let complex = kind_of(&kind)? == Kind::Complex;
            let mut out = Vec::with_capacity(n * if complex { 8 } else { 4 });
            for _ in 0..n {
                out.extend_from_slice(&((amp * ph.cos()) as f32).to_le_bytes());
                if complex {
                    out.extend_from_slice(&((amp * ph.sin()) as f32).to_le_bytes());
                }
                ph = (ph + w) % std::f64::consts::TAU;
            }
            Ok((lua.create_string(out)?, ph))
        },
    )?)?;

    // Packed real -> 16-bit PCM; PCM (8/16-bit, channels) -> packed real (mono).
    m.set("pcm16", lua.create_function(|lua, data: Value| {
        str_of(lua, audio::to_pcm16(&bytes_to_f32(&bytes(&data, "data")?)))
    })?)?;
    m.set("from_pcm", lua.create_function(|lua, (data, bits, channels): (Value, u8, u8)| {
        str_of(lua, f32_to_bytes(&audio::from_pcm(&bytes(&data, "data")?, bits, channels)))
    })?)?;
    // 44-byte header of a 16-bit mono WAV with `samples` samples.
    m.set("wav_header", lua.create_function(|lua, (rate, samples): (u32, u32)| {
        str_of(lua, audio::wav_header(rate, samples))
    })?)?;
    // rate, packed real of a WAV file's bytes (headerless = 16-bit PCM at raw_rate).
    m.set("read_audio", lua.create_function(|lua, (data, raw_rate): (Value, u32)| {
        let a = audio::read_audio(&bytes(&data, "data")?, raw_rate).map_err(err)?;
        Ok((a.rate, lua.create_string(f32_to_bytes(&a.samples))?))
    })?)?;
    // Like read_audio for the file `path`: headerless data only from a .pcm/.raw file.
    m.set("read_audio_file", lua.create_function(|lua, (path, data, raw_rate): (String, Value, u32)| {
        let a = audio::read_audio_file(&path, &bytes(&data, "data")?, raw_rate).map_err(|e| err(format!("{path}: {e}")))?;
        Ok((a.rate, lua.create_string(f32_to_bytes(&a.samples))?))
    })?)?;

    // SigMF meta JSON, and {datatype, rate, freq, description, timestamp, hw} from it.
    m.set("sigmf_meta", lua.create_function(
        |_, (datatype, rate, freq, description, timestamp, hw): (String, f64, f64, String, Option<f64>, String)| {
            let m = Meta {
                format: sample_format(&datatype)?,
                sample_rate: rate,
                frequency: freq,
                description,
                start_timestamp: timestamp.map(|t| t as i64),
                hw,
            };
            Ok(m.to_json())
        },
    )?)?;
    m.set("sigmf_parse", lua.create_function(|lua, text: String| {
        let m = Meta::parse(&text).map_err(err)?;
        let d = lua.create_table()?;
        d.set("datatype", m.format.name())?;
        d.set("rate", m.sample_rate)?;
        d.set("freq", m.frequency)?;
        d.set("description", m.description)?;
        d.set("hw", m.hw)?;
        d.set("timestamp", m.start_timestamp.map(|t| t as f64))?;
        Ok(d)
    })?)?;

    // Mean power of a stream in dB (complex |x|^2 or real x^2).
    m.set("power_db", lua.create_function(|_, (kind, data): (String, Value)| {
        let p = match to_buf(kind_of(&kind)?, &data)? {
            Buf::C(x) => ecm_dsp::complex::mean_power(&x),
            Buf::R(x) => ecm_dsp::complex::mean_power_real(&x),
            Buf::F(_) => return Err(err("frames have no power")),
        };
        Ok(ecm_dsp::complex::to_db(p) as f64)
    })?)?;

    // { {freq, power_db, snr_db}, ... } of signals in a complex block.
    m.set("find_signals", lua.create_function(
        |lua, (data, rate, center, nfft, threshold): (Value, f64, f64, usize, f64)| {
            if !nfft.is_power_of_two() || nfft < 16 {
                return Err(err("nfft must be a power of two >= 16"));
            }
            let x = bytes_to_c32(&bytes(&data, "data")?);
            let hits = ecm_radio::spectrum::find_signals(&x, rate, center, nfft, threshold as f32);
            let out = lua.create_table()?;
            for (i, h) in hits.into_iter().enumerate() {
                out.raw_set(i + 1, lua.create_sequence_from([h.freq, h.power_db as f64, h.snr_db as f64])?)?;
            }
            Ok(out)
        },
    )?)?;

    // Packed samples -> a list: complex as {re, im} pairs, real as numbers.
    m.set("to_list", lua.create_function(|lua, (kind, data): (String, Value)| {
        let out = lua.create_table()?;
        match to_buf(kind_of(&kind)?, &data)? {
            Buf::C(x) => {
                for (i, c) in x.into_iter().enumerate() {
                    out.raw_set(i + 1, lua.create_sequence_from([c.re as f64, c.im as f64])?)?;
                }
            }
            Buf::R(x) => {
                for (i, v) in x.into_iter().enumerate() {
                    out.raw_set(i + 1, v as f64)?;
                }
            }
            Buf::F(f) => {
                for (i, v) in f.into_iter().enumerate() {
                    out.raw_set(i + 1, lua.create_string(v)?)?;
                }
            }
        }
        Ok(out)
    })?)?;

    // A list of numbers ({re, im} pairs or plain numbers) -> packed samples of `kind`.
    m.set("from_list", lua.create_function(|lua, (kind, values): (String, Table)| {
        let kind = kind_of(&kind)?;
        let mut out = Vec::new();
        for v in values.sequence_values::<Value>() {
            let (re, im) = match v? {
                Value::Table(t) => (t.get::<f64>(1)?, t.get::<Option<f64>>(2)?.unwrap_or(0.0)),
                Value::Integer(i) => (i as f64, 0.0),
                Value::Number(n) => (n, 0.0),
                other => return Err(err(format!("expected numbers, got a {}", other.type_name()))),
            };
            out.extend_from_slice(&(re as f32).to_le_bytes());
            if kind == Kind::Complex {
                out.extend_from_slice(&(im as f32).to_le_bytes());
            }
        }
        str_of(lua, out)
    })?)?;

    // ---- framed PHY mode (ecm_radio::framed)

    // frames_path, ctl_path for a framed radio name.
    m.set("resolve_framed", lua.create_function(|_, name: String| {
        let p = ecm_radio::framed::resolve(&name);
        Ok((p.frames, p.ctl))
    })?)?;
    // A TX record: framed_tx(dst, payload, harq, slot, cqi, subchannel, tag).
    m.set("framed_tx", lua.create_function(
        |lua, (dst, payload, harq, slot, cqi, subchannel, tag): (f64, Value, bool, f64, u8, u8, u16)| {
            let p = bytes(&payload, "payload")?;
            if p.len() > ecm_radio::framed::MAX_PAYLOAD {
                return Err(err(format!("payload over {} bytes", ecm_radio::framed::MAX_PAYLOAD)));
            }
            let o = ecm_radio::framed::TxOpts { harq, slot: slot as u64, cqi, subchannel, tag };
            str_of(lua, ecm_radio::framed::encode_tx(dst as u32, &p, &o))
        },
    )?)?;
    // A BEACON record: framed_beacon(index, period_ms, payload).
    m.set("framed_beacon", lua.create_function(|lua, (index, period_ms, payload): (u8, u32, Value)| {
        str_of(lua, ecm_radio::framed::encode_beacon(index, period_ms, 0, 0, &bytes(&payload, "payload")?))
    })?)?;
    // The records of one read as tables (see the Python _radio.framed_parse).
    m.set("framed_parse", lua.create_function(|lua, data: Value| {
        use ecm_radio::framed::Record;
        let out = lua.create_table()?;
        for (i, r) in ecm_radio::framed::parse_records(&bytes(&data, "data")?).into_iter().enumerate() {
            let d = lua.create_table()?;
            match r {
                Record::Rx(x) => {
                    d.set("type", "rx")?;
                    d.set("src", x.src as f64)?;
                    d.set("dst", x.dst as f64)?;
                    d.set("seq", x.seq as f64)?;
                    d.set("flags", x.flags as f64)?;
                    d.set("beacon", x.is_beacon())?;
                    d.set("cqi", x.cqi as f64)?;
                    d.set("cqi_sent", x.cqi_sent as f64)?;
                    d.set("rssi", x.rssi_dbm as f64)?;
                    d.set("sinr", x.sinr_db as f64)?;
                    d.set("timestamp_us", x.timestamp_us as f64)?;
                    d.set("slot", x.slot as f64)?;
                    d.set("subchannel", x.subchannel as f64)?;
                    d.set("cqi_report", x.cqi_report as f64)?;
                    d.set("delay_us", x.delay_us as f64)?;
                    d.set("payload", lua.create_string(x.payload)?)?;
                }
                Record::Status(s) => {
                    d.set("type", "status")?;
                    d.set("result", s.result_name())?;
                    d.set("dst", s.dst as f64)?;
                    d.set("tag", s.tag as f64)?;
                    d.set("attempts", s.attempts as f64)?;
                    d.set("cqi", s.cqi as f64)?;
                    d.set("slot", s.slot as f64)?;
                    d.set("airtime_us", s.airtime_us as f64)?;
                }
            }
            out.raw_set(i + 1, d)?;
        }
        Ok(out)
    })?)?;

    // data_path, ctl_path for an SDR name.
    m.set("resolve_sdr", lua.create_function(|_, name: String| {
        let p = resolve(&name);
        Ok((p.data, p.ctl))
    })?)?;

    // `key value` lines -> table (numbers converted).
    m.set("parse_status", lua.create_function(|lua, text: String| {
        let d = lua.create_table()?;
        for (k, v) in Status::parse(&text).0 {
            if let Ok(f) = v.parse::<f64>() {
                d.set(k, f)?;
            } else {
                d.set(k, v)?;
            }
        }
        Ok(d)
    })?)?;

    // "144.39M" -> 144390000 (numbers pass through).
    m.set("parse_freq", lua.create_function(|_, value: Value| match value {
        Value::String(s) => {
            let s = s.to_str()?.to_string();
            ecm_radio::units::parse_freq(&s).ok_or_else(|| err(format!("bad frequency {s:?}")))
        }
        Value::Integer(i) => Ok(i as f64),
        Value::Number(n) => Ok(n),
        other => Err(err(format!("bad frequency (a {})", other.type_name()))),
    })?)?;

    // AX.25 UI frame body (no FCS).
    m.set("ax25_encode", lua.create_function(|lua, (dest, src, info, pid): (String, String, Value, u8)| {
        let d = Address::parse(&dest).map_err(|e| err(format!("destination: {e}")))?;
        let s = Address::parse(&src).map_err(|e| err(format!("source: {e}")))?;
        let mut f = UiFrame::new(d, s, &bytes(&info, "info")?);
        f.pid = pid;
        str_of(lua, f.encode().map_err(|e| err(e.to_string()))?)
    })?)?;
    // {dest, src, digis, pid, info, text} of an AX.25 UI frame, or nil.
    m.set("ax25_decode", lua.create_function(|lua, data: Value| {
        let Ok(f) = UiFrame::parse(&bytes(&data, "frame")?) else {
            return Ok(MultiValue::from_iter([Value::Nil]));
        };
        let d = lua.create_table()?;
        d.set("dest", f.dest.to_string())?;
        d.set("src", f.src.to_string())?;
        d.set("digis", lua.create_sequence_from(f.digis.iter().map(|a| a.to_string()))?)?;
        d.set("pid", f.pid as f64)?;
        d.set("text", String::from_utf8_lossy(&f.info).into_owned())?;
        d.set("info", lua.create_string(&f.info)?)?;
        Ok(MultiValue::from_iter([Value::Table(d)]))
    })?)?;

    Ok(m)
}
