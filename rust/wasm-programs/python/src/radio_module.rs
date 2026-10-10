//! `_radio` native module: the per-sample work behind the Python `radio`
//! module (radio.py), on top of `ecm-radio` / `ecm-dsp`. RustPython has no
//! numpy, so samples cross into Python as packed bytes (little-endian f32:
//! interleaved I/Q for complex streams, one value per sample for real
//! streams) and frames as lists of `bytes`; Python only wires blocks.
//!
//! Chains are kept in a per-thread table and addressed by integer handles.

use ecm_radio::blocks::{Buf, Chain, Kind, Param, Spec};
use ecm_radio::C32;
use rustpython_vm::{
    builtins::{PyBytes, PyComplex, PyDict, PyFloat, PyInt, PyList, PyStr, PyTuple},
    pymodule, AsObject, PyObjectRef, PyResult, VirtualMachine,
};
use std::cell::RefCell;
use std::collections::BTreeMap;

/// Source of the Python-level `radio` module.
pub const RADIO_PY: &str = include_str!("radio.py");

thread_local! {
    static CHAINS: RefCell<(i64, BTreeMap<i64, Chain>)> = RefCell::new((0, BTreeMap::new()));
}

pub fn c32_to_bytes(x: &[C32]) -> Vec<u8> {
    let mut v = Vec::with_capacity(x.len() * 8);
    for s in x {
        v.extend_from_slice(&s.re.to_le_bytes());
        v.extend_from_slice(&s.im.to_le_bytes());
    }
    v
}

pub fn bytes_to_c32(b: &[u8]) -> Vec<C32> {
    b.chunks_exact(8)
        .map(|c| C32::new(f32::from_le_bytes([c[0], c[1], c[2], c[3]]), f32::from_le_bytes([c[4], c[5], c[6], c[7]])))
        .collect()
}

pub fn f32_to_bytes(x: &[f32]) -> Vec<u8> {
    x.iter().flat_map(|v| v.to_le_bytes()).collect()
}

pub fn bytes_to_f32(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn err(vm: &VirtualMachine, msg: impl Into<String>) -> rustpython_vm::builtins::PyBaseExceptionRef {
    vm.new_value_error(msg.into())
}

fn as_bytes(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
    if let Some(b) = obj.payload::<PyBytes>() {
        return Ok(b.as_bytes().to_vec());
    }
    if let Some(s) = obj.payload::<PyStr>() {
        return Ok(s.as_str().as_bytes().to_vec());
    }
    if let Some(ba) = obj.payload::<rustpython_vm::builtins::PyByteArray>() {
        return Ok(ba.borrow_buf().to_vec());
    }
    Err(vm.new_type_error(format!("expected bytes, got {}", obj.class().name())))
}

fn as_f64(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<f64> {
    Ok(obj.try_float(vm)?.to_f64())
}

fn as_str(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
    match obj.payload::<PyStr>() {
        Some(s) => Ok(s.as_str().to_owned()),
        None => Err(vm.new_type_error(format!("expected str, got {}", obj.class().name()))),
    }
}

fn kind_of(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<Kind> {
    let s = as_str(obj, vm)?;
    Kind::parse(&s).ok_or_else(|| err(vm, format!("unknown stream kind {s:?} (complex, real, frames)")))
}

fn to_buf(kind: Kind, data: &PyObjectRef, vm: &VirtualMachine) -> PyResult<Buf> {
    Ok(match kind {
        Kind::Complex => Buf::C(bytes_to_c32(&as_bytes(data, vm)?)),
        Kind::Real => Buf::R(bytes_to_f32(&as_bytes(data, vm)?)),
        Kind::Frames => {
            let items: Vec<PyObjectRef> = if let Some(l) = data.payload::<PyList>() {
                l.borrow_vec().to_vec()
            } else if let Some(t) = data.payload::<PyTuple>() {
                t.as_slice().to_vec()
            } else {
                return Err(vm.new_type_error("frames must be a list of bytes".to_owned()));
            };
            Buf::F(items.iter().map(|o| as_bytes(o, vm)).collect::<PyResult<_>>()?)
        }
    })
}

fn from_buf(b: Buf, vm: &VirtualMachine) -> PyObjectRef {
    match b {
        Buf::C(x) => vm.ctx.new_bytes(c32_to_bytes(&x)).into(),
        Buf::R(x) => vm.ctx.new_bytes(f32_to_bytes(&x)).into(),
        Buf::F(f) => vm.ctx.new_list(f.into_iter().map(|v| vm.ctx.new_bytes(v).into()).collect()).into(),
    }
}

fn param_of(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<Param> {
    if let Some(s) = obj.payload::<PyStr>() {
        return Ok(Param::Str(s.as_str().to_owned()));
    }
    if obj.class().is(vm.ctx.types.bool_type) {
        return Ok(Param::Num(if obj.is(&vm.ctx.true_value) { 1.0 } else { 0.0 }));
    }
    if obj.payload::<PyInt>().is_some() || obj.payload::<PyFloat>().is_some() {
        return Ok(Param::Num(as_f64(obj, vm)?));
    }
    Err(vm.new_type_error(format!("block parameters must be numbers or strings, got {}", obj.class().name())))
}

/// The `_radio` native module for the interpreter's module table (`#[pymodule]`
/// makes `radio_native::make_module` crate-private, and the binary is another crate).
pub fn make_native_module(vm: &VirtualMachine) -> rustpython_vm::PyRef<rustpython_vm::builtins::PyModule> {
    radio_native::make_module(vm)
}

#[pymodule]
pub mod radio_native {
    use super::*;
    use ecm_radio::audio;
    use ecm_radio::device::{resolve, Status};
    use ecm_radio::sigmf::{format_from_datatype, Meta};
    use ecm_dsp::coding::ax25::{Address, UiFrame};
    use ecm_dsp::iq::SampleFormat;

    /// Source of the user-facing module (see radio.py).
    #[pyattr]
    fn _source(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(RADIO_PY).into()
    }

    /// Build a chain: `blocks` = [(name, {param: value}), ...]; returns
    /// (handle, out_kind, out_rate, description).
    #[pyfunction]
    fn chain_new(blocks: PyObjectRef, kind: PyObjectRef, rate: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let kind = kind_of(&kind, vm)?;
        let rate = as_f64(&rate, vm)?;
        let items: Vec<PyObjectRef> = match blocks.payload::<PyList>() {
            Some(l) => l.borrow_vec().to_vec(),
            None => return Err(vm.new_type_error("blocks must be a list".to_owned())),
        };
        let mut specs = Vec::new();
        for it in items {
            let Some(t) = it.payload::<PyTuple>() else {
                return Err(vm.new_type_error("each block is (name, params)".to_owned()));
            };
            let [name, params] = t.as_slice() else {
                return Err(vm.new_type_error("each block is (name, params)".to_owned()));
            };
            let name = as_str(name, vm)?;
            let mut p = BTreeMap::new();
            if let Some(d) = params.payload::<PyDict>() {
                for (k, v) in d {
                    if vm.is_none(&v) {
                        continue;
                    }
                    p.insert(as_str(&k, vm)?, param_of(&v, vm)?);
                }
            }
            specs.push(Spec::from_params(&name, &p).map_err(|e| err(vm, e))?);
        }
        let chain = Chain::build(specs, kind, rate).map_err(|e| err(vm, e))?;
        let (ok, orate, desc) = (chain.out_kind(), chain.out_rate(), chain.describe());
        let h = CHAINS.with(|c| {
            let mut c = c.borrow_mut();
            c.0 += 1;
            let h = c.0;
            c.1.insert(h, chain);
            h
        });
        Ok(vm
            .ctx
            .new_tuple(vec![
                vm.ctx.new_int(h).into(),
                vm.ctx.new_str(ok.name()).into(),
                vm.ctx.new_float(orate).into(),
                vm.ctx.new_str(desc).into(),
            ])
            .into())
    }

    /// Run one buffer through a chain; returns its output (bytes or list of bytes).
    #[pyfunction]
    fn chain_process(handle: i64, kind: PyObjectRef, data: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let buf = to_buf(kind_of(&kind, vm)?, &data, vm)?;
        let out = CHAINS.with(|c| {
            let mut c = c.borrow_mut();
            match c.1.get_mut(&handle) {
                Some(ch) => ch.process(buf),
                None => Err(format!("no chain {handle}")),
            }
        });
        out.map(|b| from_buf(b, vm)).map_err(|e| err(vm, e))
    }

    #[pyfunction]
    fn chain_free(handle: i64) {
        CHAINS.with(|c| c.borrow_mut().1.remove(&handle));
    }

    fn sample_format(name: &PyObjectRef, vm: &VirtualMachine) -> PyResult<SampleFormat> {
        let s = as_str(name, vm)?;
        format_from_datatype(&s).ok_or_else(|| err(vm, format!("unknown sample format {s:?}")))
    }

    /// Device / file bytes in `fmt` (cs16, cf32, ...) -> packed complex.
    #[pyfunction]
    fn decode(fmt: PyObjectRef, data: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let f = sample_format(&fmt, vm)?;
        let mut out = Vec::new();
        f.decode(&as_bytes(&data, vm)?, &mut out);
        Ok(vm.ctx.new_bytes(c32_to_bytes(&out)).into())
    }

    /// Packed complex -> device / file bytes in `fmt`.
    #[pyfunction]
    fn encode(fmt: PyObjectRef, data: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let f = sample_format(&fmt, vm)?;
        let mut out = Vec::new();
        f.encode(&bytes_to_c32(&as_bytes(&data, vm)?), &mut out);
        Ok(vm.ctx.new_bytes(out).into())
    }

    /// Bytes per sample of a format.
    #[pyfunction]
    fn sample_size(fmt: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        Ok(sample_format(&fmt, vm)?.bytes_per_sample())
    }

    /// n samples of a tone at `hz` starting at `phase` (radians); returns
    /// (data, next_phase). `kind` complex = e^{j w t}, real = cos.
    #[pyfunction]
    fn tone(
        hz: PyObjectRef,
        rate: PyObjectRef,
        n: usize,
        amplitude: PyObjectRef,
        phase: PyObjectRef,
        kind: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let (hz, rate, amp, mut ph) = (as_f64(&hz, vm)?, as_f64(&rate, vm)?, as_f64(&amplitude, vm)?, as_f64(&phase, vm)?);
        if rate <= 0.0 {
            return Err(err(vm, "rate must be > 0"));
        }
        let w = std::f64::consts::TAU * hz / rate;
        let complex = kind_of(&kind, vm)? == Kind::Complex;
        let mut out = Vec::with_capacity(n * if complex { 8 } else { 4 });
        for _ in 0..n {
            if complex {
                out.extend_from_slice(&((amp * ph.cos()) as f32).to_le_bytes());
                out.extend_from_slice(&((amp * ph.sin()) as f32).to_le_bytes());
            } else {
                out.extend_from_slice(&((amp * ph.cos()) as f32).to_le_bytes());
            }
            ph = (ph + w) % std::f64::consts::TAU;
        }
        Ok(vm.ctx.new_tuple(vec![vm.ctx.new_bytes(out).into(), vm.ctx.new_float(ph).into()]).into())
    }

    /// Packed real -> 16-bit PCM.
    #[pyfunction]
    fn pcm16(data: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(vm.ctx.new_bytes(audio::to_pcm16(&bytes_to_f32(&as_bytes(&data, vm)?))).into())
    }

    /// PCM (8/16-bit, channels) -> packed real (mono).
    #[pyfunction]
    fn from_pcm(data: PyObjectRef, bits: u8, channels: u8, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(vm.ctx.new_bytes(f32_to_bytes(&audio::from_pcm(&as_bytes(&data, vm)?, bits, channels))).into())
    }

    /// 44-byte header of a 16-bit mono WAV with `samples` samples.
    #[pyfunction]
    fn wav_header(rate: u32, samples: u32, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_bytes(audio::wav_header(rate, samples)).into()
    }

    /// (rate, packed real) of a WAV file's bytes (headerless = 16-bit PCM at raw_rate).
    #[pyfunction]
    fn read_audio(data: PyObjectRef, raw_rate: u32, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let a = audio::read_audio(&as_bytes(&data, vm)?, raw_rate);
        Ok(vm
            .ctx
            .new_tuple(vec![vm.ctx.new_int(a.rate).into(), vm.ctx.new_bytes(f32_to_bytes(&a.samples)).into()])
            .into())
    }

    /// SigMF meta JSON.
    #[pyfunction]
    fn sigmf_meta(
        datatype: PyObjectRef,
        rate: PyObjectRef,
        freq: PyObjectRef,
        description: PyObjectRef,
        timestamp: PyObjectRef,
        hw: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        let m = Meta {
            format: sample_format(&datatype, vm)?,
            sample_rate: as_f64(&rate, vm)?,
            frequency: as_f64(&freq, vm)?,
            description: as_str(&description, vm)?,
            start_timestamp: if vm.is_none(&timestamp) { None } else { Some(as_f64(&timestamp, vm)? as i64) },
            hw: as_str(&hw, vm)?,
        };
        Ok(m.to_json())
    }

    /// {datatype, rate, freq, description, timestamp, hw} from SigMF meta JSON.
    #[pyfunction]
    fn sigmf_parse(text: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let m = Meta::parse(&as_str(&text, vm)?).map_err(|e| err(vm, e))?;
        let d = vm.ctx.new_dict();
        d.set_item("datatype", vm.ctx.new_str(m.format.name()).into(), vm)?;
        d.set_item("rate", vm.ctx.new_float(m.sample_rate).into(), vm)?;
        d.set_item("freq", vm.ctx.new_float(m.frequency).into(), vm)?;
        d.set_item("description", vm.ctx.new_str(m.description).into(), vm)?;
        d.set_item("hw", vm.ctx.new_str(m.hw).into(), vm)?;
        let ts = match m.start_timestamp {
            Some(t) => vm.ctx.new_int(t).into(),
            None => vm.ctx.none(),
        };
        d.set_item("timestamp", ts, vm)?;
        Ok(d.into())
    }

    /// Mean power of a stream in dB (complex |x|^2 or real x^2).
    #[pyfunction]
    fn power_db(kind: PyObjectRef, data: PyObjectRef, vm: &VirtualMachine) -> PyResult<f64> {
        let p = match to_buf(kind_of(&kind, vm)?, &data, vm)? {
            Buf::C(x) => ecm_dsp::complex::mean_power(&x),
            Buf::R(x) => ecm_dsp::complex::mean_power_real(&x),
            Buf::F(_) => return Err(err(vm, "frames have no power")),
        };
        Ok(ecm_dsp::complex::to_db(p) as f64)
    }

    /// [(freq, power_db, snr_db)] of signals in a complex block.
    #[pyfunction]
    fn find_signals(
        data: PyObjectRef,
        rate: PyObjectRef,
        center: PyObjectRef,
        nfft: usize,
        threshold: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        if !nfft.is_power_of_two() || nfft < 16 {
            return Err(err(vm, "nfft must be a power of two >= 16"));
        }
        let x = bytes_to_c32(&as_bytes(&data, vm)?);
        let hits = ecm_radio::spectrum::find_signals(&x, as_f64(&rate, vm)?, as_f64(&center, vm)?, nfft, as_f64(&threshold, vm)? as f32);
        let objs = hits
            .into_iter()
            .map(|h| {
                vm.ctx
                    .new_tuple(vec![
                        vm.ctx.new_float(h.freq).into(),
                        vm.ctx.new_float(h.power_db as f64).into(),
                        vm.ctx.new_float(h.snr_db as f64).into(),
                    ])
                    .into()
            })
            .collect();
        Ok(vm.ctx.new_list(objs).into())
    }

    /// Packed samples -> list of Python complex (complex) or float (real).
    #[pyfunction]
    fn to_list(kind: PyObjectRef, data: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let objs = match to_buf(kind_of(&kind, vm)?, &data, vm)? {
            Buf::C(x) => x
                .into_iter()
                .map(|c| vm.ctx.new_complex(num_complex::Complex64::new(c.re as f64, c.im as f64)).into())
                .collect(),
            Buf::R(x) => x.into_iter().map(|v| vm.ctx.new_float(v as f64).into()).collect(),
            Buf::F(f) => f.into_iter().map(|v| vm.ctx.new_bytes(v).into()).collect(),
        };
        Ok(vm.ctx.new_list(objs).into())
    }

    /// List of numbers (complex or real) -> packed samples of `kind`.
    #[pyfunction]
    fn from_list(kind: PyObjectRef, values: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let kind = kind_of(&kind, vm)?;
        let items: Vec<PyObjectRef> = if let Some(l) = values.payload::<PyList>() {
            l.borrow_vec().to_vec()
        } else if let Some(t) = values.payload::<PyTuple>() {
            t.as_slice().to_vec()
        } else {
            return Err(vm.new_type_error("expected a list of numbers".to_owned()));
        };
        let mut out = Vec::with_capacity(items.len() * 8);
        for o in items {
            let (re, im) = match o.payload::<PyComplex>() {
                Some(c) => {
                    let v = c.to_complex();
                    (v.re, v.im)
                }
                None => (as_f64(&o, vm)?, 0.0),
            };
            out.extend_from_slice(&(re as f32).to_le_bytes());
            if kind == Kind::Complex {
                out.extend_from_slice(&(im as f32).to_le_bytes());
            }
        }
        Ok(vm.ctx.new_bytes(out).into())
    }

    /// (data_path, ctl_path) for an SDR name.
    #[pyfunction]
    fn resolve_sdr(name: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let p = resolve(&as_str(&name, vm)?);
        Ok(vm.ctx.new_tuple(vec![vm.ctx.new_str(p.data).into(), vm.ctx.new_str(p.ctl).into()]).into())
    }

    /// `key value` lines -> dict (numbers converted).
    #[pyfunction]
    fn parse_status(text: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let s = Status::parse(&as_str(&text, vm)?);
        let d = vm.ctx.new_dict();
        for (k, v) in s.0 {
            let obj: PyObjectRef = if let Ok(i) = v.parse::<i64>() {
                vm.ctx.new_int(i).into()
            } else if let Ok(f) = v.parse::<f64>() {
                vm.ctx.new_float(f).into()
            } else {
                vm.ctx.new_str(v).into()
            };
            d.set_item(k.as_str(), obj, vm)?;
        }
        Ok(d.into())
    }

    /// `144.39M` -> 144390000.0 (numbers pass through).
    #[pyfunction]
    fn parse_freq(value: PyObjectRef, vm: &VirtualMachine) -> PyResult<f64> {
        if let Some(s) = value.payload::<PyStr>() {
            return ecm_radio::units::parse_freq(s.as_str()).ok_or_else(|| err(vm, format!("bad frequency {:?}", s.as_str())));
        }
        as_f64(&value, vm)
    }

    /// AX.25 UI frame body (no FCS).
    #[pyfunction]
    fn ax25_encode(dest: PyObjectRef, src: PyObjectRef, info: PyObjectRef, pid: u8, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let d = Address::parse(&as_str(&dest, vm)?).map_err(|e| err(vm, format!("destination: {e}")))?;
        let s = Address::parse(&as_str(&src, vm)?).map_err(|e| err(vm, format!("source: {e}")))?;
        let mut f = UiFrame::new(d, s, &as_bytes(&info, vm)?);
        f.pid = pid;
        Ok(vm.ctx.new_bytes(f.encode().map_err(|e| err(vm, e.to_string()))?).into())
    }

    /// {dest, src, digis, pid, info, text} of an AX.25 UI frame, or None.
    #[pyfunction]
    fn ax25_decode(data: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let Ok(f) = UiFrame::parse(&as_bytes(&data, vm)?) else {
            return Ok(vm.ctx.none());
        };
        let d = vm.ctx.new_dict();
        d.set_item("dest", vm.ctx.new_str(f.dest.to_string()).into(), vm)?;
        d.set_item("src", vm.ctx.new_str(f.src.to_string()).into(), vm)?;
        let digis = f.digis.iter().map(|a| vm.ctx.new_str(a.to_string()).into()).collect();
        d.set_item("digis", vm.ctx.new_list(digis).into(), vm)?;
        d.set_item("pid", vm.ctx.new_int(f.pid).into(), vm)?;
        d.set_item("text", vm.ctx.new_str(String::from_utf8_lossy(&f.info).into_owned()).into(), vm)?;
        d.set_item("info", vm.ctx.new_bytes(f.info).into(), vm)?;
        Ok(d.into())
    }
}

#[cfg(test)]
mod tests;
