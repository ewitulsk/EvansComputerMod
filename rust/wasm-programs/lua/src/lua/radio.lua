--[[ Software radio flowgraphs: SDR blocks, demodulators, modems and sinks.
The Lua counterpart of Python's `radio` module (same blocks and functions).

    local sdr = radio.open("sdr_0")        -- or an attachment name, or an index
    sdr:tune(146.52e6, 48000)              -- freq, rate, bw
    local fg = radio.Flowgraph(sdr .. radio.fm_demod(5e3) .. radio.lowpass(3e3) .. radio.speaker())
    fg:run{seconds = 10}

Pipelines are written with `..` (Python's `>>`); radio.pipeline(a, b, c) works too.

Streams are complex baseband, real audio, or frames (packet bodies). The
heavy per-sample work runs natively (`_radio`, ecm-dsp); this module only
wires blocks together. Samples travel as radio.Samples (packed float32 in a
string); complex values in lists are {re, im} pairs.

Sources: radio.open(...) (an SDR), tone(), file_source() (.cf32/.cs16 with
SigMF metadata), wav_source(), frames_source().
Blocks:  fm_demod, wbfm_demod, fm_mod, am_demod, am_mod, ssb_demod(mode),
         ssb_mod(mode), lowpass, resample, decimate, agc, squelch, shift, gain,
         dc_block, real, to_complex, afsk1200, fsk, bpsk, chirp.
         Packet modems modulate when fed frames and demodulate when fed samples.
Sinks:   speaker(), wav_sink(path), iq_sink(path), collect(), frames(), or an
         SDR (transmit; call sdr:tx(true) first; closing it switches tx off).
Framed:  radio.framed(name) -> an Advanced SDR (or a modem module) in framed
         PHY mode: configure{...}, send(), recv(), beacon().

Optional arguments go in Python's order, or as one table of names:
radio.tone(1000, 8000, 0.5, 1.0) is radio.tone(1000, {rate=8000, seconds=1.0}).

Flowgraph:run() ends when the source ends, at a seconds=/samples= limit, after
`timeout` seconds without input (10 s by default: a silent SDR; timeout=false
waits forever), or on stop().
]]

local native = require("_radio")
local ecm = require("_ecm")
local unpack = table.unpack or unpack

local M = {}

M.COMPLEX = "complex"
M.REAL = "real"
M.FRAMES = "frames"
local COMPLEX, REAL, FRAMES = M.COMPLEX, M.REAL, M.FRAMES

local function radio_error(msg, level)
  error("RadioError: " .. msg, (level or 1) + 1)
end
M.error = radio_error

-- Optional arguments: positional in `names` order, or one table of names.
local function args(names, ...)
  local n = select("#", ...)
  local first = ...
  local out = {}
  if n == 1 and type(first) == "table" and getmetatable(first) == nil then
    for _, k in ipairs(names) do out[k] = first[k] end
  else
    local a = { ... }
    for i, k in ipairs(names) do out[k] = a[i] end
  end
  return out
end

local function class(base)
  local c = {}
  c.__index = c
  if base then setmetatable(c, { __index = base }) end
  return c
end

local function is_a(x, c)
  local mt = getmetatable(x)
  while mt do
    if mt == c then return true end
    local up = getmetatable(mt)
    mt = up and up.__index
  end
  return false
end

-- ---------------------------------------------------------------- Samples

local Samples = class()
M.Samples = Samples

local function samples(kind, data, rate)
  return setmetatable({ kind = kind, data = data, rate = rate }, Samples)
end
M.new_samples = samples

function Samples:count()
  if self.kind == FRAMES then return #self.data end
  return math.floor(#self.data / (self.kind == COMPLEX and 8 or 4))
end
Samples.__len = Samples.count

--- {re, im} pairs (complex), numbers (real) or strings (frames).
function Samples:to_list()
  if self.kind == FRAMES then
    local out = {}
    for i, f in ipairs(self.data) do out[i] = f end
    return out
  end
  return native.to_list(self.kind, self.data)
end

function Samples:power_db()
  return native.power_db(self.kind, self.data)
end

Samples.__tostring = function(s)
  return string.format("<Samples %s x%d @ %g>", s.kind, s:count(), s.rate)
end

function Samples.from_list(values, rate, kind)
  kind = kind or COMPLEX
  return samples(kind, native.from_list(kind, values), rate)
end

local function as_samples(x, rate, kind)
  kind = kind or COMPLEX
  if is_a(x, Samples) then return x end
  if type(x) == "string" then return samples(kind, x, rate) end
  return Samples.from_list(x, rate, kind)
end

-- ---------------------------------------------------------------- pipelines

local Node = class()
local Pipeline = class()
M.Pipeline = Pipeline

local function items_of(x)
  if is_a(x, Pipeline) then return x.items end
  return { x }
end

local function pipeline(list)
  return setmetatable({ items = list }, Pipeline)
end

local function concat(a, b)
  local out = {}
  for _, x in ipairs(items_of(a)) do out[#out + 1] = x end
  for _, x in ipairs(items_of(b)) do out[#out + 1] = x end
  return pipeline(out)
end
Node.__concat = concat
Pipeline.__concat = concat
Pipeline.__tostring = function(p)
  local parts = {}
  for i, x in ipairs(p.items) do parts[i] = tostring(x) end
  return table.concat(parts, " .. ")
end

--- radio.pipeline(a, b, c) is a .. b .. c.
function M.pipeline(...)
  local out = {}
  for _, x in ipairs({ ... }) do
    for _, y in ipairs(items_of(x)) do out[#out + 1] = y end
  end
  return pipeline(out)
end

-- Classes derived from Node keep its metamethods.
local function node_class(base)
  local c = class(base or Node)
  c.__concat = concat
  c.__tostring = (base or Node).__tostring
  return c
end

-- ---------------------------------------------------------------- blocks

local Block = node_class()
M.Block = Block

local function block(name, params)
  return setmetatable({ name = name, params = params or {} }, Block)
end
M.new_block = block

function Block:spec()
  return { self.name, self.params }
end

Block.__tostring = function(b)
  local keys = {}
  for k in pairs(b.params) do keys[#keys + 1] = k end
  table.sort(keys)
  local parts = {}
  for i, k in ipairs(keys) do parts[i] = k .. "=" .. tostring(b.params[k]) end
  return b.name .. "(" .. table.concat(parts, ", ") .. ")"
end

local Chain = class()

local function chain(blocks, kind, rate)
  local specs = {}
  for i, b in ipairs(blocks) do specs[i] = b:spec() end
  local c = setmetatable({}, Chain)
  c.handle, c.kind, c.rate, c.description = native.chain_new(specs, kind, rate)
  c.in_kind = kind
  return c
end

function Chain:process(s)
  if s.kind ~= self.in_kind then
    radio_error(string.format("chain expects %s samples, got %s", self.in_kind, s.kind))
  end
  return samples(self.kind, native.chain_process(self.handle, s.kind, s.data), self.rate)
end

function Chain:close()
  if self.handle ~= nil then
    native.chain_free(self.handle)
    self.handle = nil
  end
end

--- FM discriminator (complex -> real). `tau`: de-emphasis, e.g. 75e-6.
function M.fm_demod(deviation, tau) return block("fm_demod", { deviation = deviation or 5e3, tau = tau or 0.0 }) end
function M.wbfm_demod(deviation, tau) return block("fm_demod", { deviation = deviation or 75e3, tau = tau or 75e-6 }) end
--- FM modulator (real -> complex).
function M.fm_mod(deviation, tau) return block("fm_mod", { deviation = deviation or 5e3, tau = tau or 0.0 }) end
function M.am_demod() return block("am_demod") end
function M.am_mod(index) return block("am_mod", { index = index or 0.8 }) end
function M.ssb_demod(mode) return block("ssb_demod", { mode = mode or "usb" }) end
function M.ssb_mod(mode) return block("ssb_mod", { mode = mode or "usb" }) end
function M.lowpass(cutoff) return block("lowpass", { cutoff = cutoff }) end
function M.resample(rate) return block("resample", { rate = rate }) end
function M.decimate(factor) return block("decimate", { factor = factor }) end
function M.agc(target) return block("agc", { target = target or 0.5 }) end
--- Zero a complex stream while its power is below `threshold` dBFS.
function M.squelch(threshold) return block("squelch", { threshold = threshold or -40.0 }) end
function M.shift(hz) return block("shift", { hz = hz }) end
function M.gain(g) return block("gain", { gain = g }) end
function M.dc_block() return block("dc_block") end
function M.real() return block("real") end
function M.to_complex() return block("complex") end
--- Bell 202 AX.25 packet modem (audio). Frames in -> audio; audio in -> frames.
function M.afsk1200() return block("afsk1200") end
function M.fsk(baud, deviation) return block("fsk", { baud = baud or 1200, deviation = deviation or 2400 }) end
function M.bpsk(baud) return block("bpsk", { baud = baud or 2400 }) end
function M.chirp(sf, bw) return block("chirp", { sf = sf or 7, bw = bw or 12000 }) end

-- ---------------------------------------------------------------- files

local function open_file(path, mode, unbuffered)
  local f, e = io.open(path, mode)
  if not f then return nil, e end
  if unbuffered then f:setvbuf("no") end
  return f
end

local function read_file(path)
  local f, e = open_file(path, "rb")
  if not f then error(e, 0) end
  local s = f:read("*a") or ""
  f:close()
  return s
end

local function write_control(path, command)
  local f, e = open_file(path, "wb", true)
  if not f then error(e, 0) end
  f:write((command:gsub("^%s+", ""):gsub("%s+$", "")) .. "\n")
  f:close()
end

local function read_status(path)
  local f, e = open_file(path, "rb", true)
  if not f then error(e, 0) end
  local parts = {}
  while true do
    local chunk = f:read(4096)
    if not chunk or chunk == "" then break end
    parts[#parts + 1] = chunk
  end
  f:close()
  return native.parse_status(table.concat(parts))
end

local function meta_path(path)
  local slash = (path:match("^.*()/") or 0) + 1
  local dot = path:match("^.*()%.")
  local base = (dot and dot >= slash) and path:sub(1, dot - 1) or path
  return base .. ".sigmf-meta"
end

local function lower_ends(path, ...)
  local lower = path:lower()
  for _, suffix in ipairs({ ... }) do
    if lower:sub(-#suffix) == suffix then return true end
  end
  return false
end

-- ---------------------------------------------------------------- sources

local Source = node_class()
M.Source = Source
Source.kind = COMPLEX
Source.rate = 48000.0
Source.__tostring = function(s) return "<source " .. tostring(s.kind) .. ">" end

--- Up to n samples, or nil at the end of the stream.
function Source:read(n) error("not implemented", 2) end
function Source:close() end

local SDR = node_class(Source)
M.SDR = SDR
SDR.__tostring = function(s) return "<SDR " .. s.ctl_path .. ">" end

--- The SDR called `name`: "" (first), "sdr_0" / 0 (index), or an attachment
--- name ("left", ...). Options: data_path, ctl_path.
function M.open(name, opts)
  if type(name) == "table" then name, opts = "", name end
  opts = opts or {}
  local s = setmetatable({}, SDR)
  local data_path, ctl_path = opts.data_path, opts.ctl_path
  if data_path == nil or ctl_path == nil then
    local d, c = native.resolve_sdr(tostring(name or ""))
    data_path = data_path or d
    ctl_path = ctl_path or c
  end
  s.data_path, s.ctl_path = data_path, ctl_path
  s._sent, s._keyed, s.lead = 0, false, 0.25
  local ok, st = pcall(read_status, ctl_path)
  if not ok then radio_error(string.format("no SDR at %s: %s", ctl_path, tostring(st)), 2) end
  s.format = st.format or "cs16"
  s.rate = tonumber(st.rate) or 48000.0
  s.freq = tonumber(st.freq) or 0.0
  return s
end

function SDR:control(command) write_control(self.ctl_path, command) end

function SDR:status() return read_status(self.ctl_path) end

--- tune(freq, rate, bw); freq and bw take "146.52M"-style strings too.
function SDR:tune(freq, ...)
  local o = args({ "rate", "bw" }, ...)
  freq = native.parse_freq(freq)
  self:control(string.format("freq %d", math.floor(freq + 0.5)))
  self.freq = freq
  if o.rate ~= nil then self:set_rate(o.rate) end
  if o.bw ~= nil then self:control(string.format("bw %d", math.floor(native.parse_freq(o.bw) + 0.5))) end
  return self
end

function SDR:set_rate(rate)
  rate = math.floor(native.parse_freq(rate) + 0.5)
  self:control(string.format("rate %d", rate))
  self.rate = rate
  self._tx_start = nil
end

function SDR:gain(db) self:control("gain " .. tostring(db)) end

function SDR:agc(on)
  if on == nil or on then self:control("gain agc") else self:control("agc 0") end
end

function SDR:set_format(fmt)
  self:control("format " .. fmt)
  self.format = fmt
end

--- Transmitter on (at `power` dBm, default the tier maximum) or off.
function SDR:tx(on, power)
  if on == nil then on = true end
  if on then
    self:control(power == nil and "tx on" or ("tx on " .. tostring(power)))
  else
    self:control("tx off")
  end
  self._keyed = on and true or false
  self._tx_start = nil
  self._sent = 0
end

function SDR:timestamp()
  return math.floor(tonumber(self:status().timestamp) or 0)
end

function SDR:read(n)
  n = n or 4096
  if self._rx == nil then
    local f, e = open_file(self.data_path, "rb", true)
    if not f then radio_error(e) end
    self._rx = f
  end
  local size = native.sample_size(self.format)
  local raw = self._rx:read(n * size) or ""
  raw = raw:sub(1, #raw - #raw % size)
  return samples(COMPLEX, native.decode(self.format, raw), self.rate)
end

function SDR:prepare(kind, rate)
  if kind ~= COMPLEX then radio_error("an SDR transmits complex samples, got " .. kind) end
  if math.abs(rate - self.rate) > 0.5 then return { M.resample(self.rate) } end
  return {}
end

function SDR:write(s)
  s = as_samples(s, self.rate)
  if s.kind ~= COMPLEX then radio_error("an SDR transmits complex samples") end
  if self._tx == nil then
    local f, e = open_file(self.data_path, "wb", true)
    if not f then radio_error(e) end
    self._tx = f
  end
  self._tx:write(native.encode(self.format, s.data))
  local n = s:count()
  local now = self:timestamp()
  if self._tx_start == nil then self._tx_start = now end
  self._sent = self._sent + n
  local ahead = self._sent - (now - self._tx_start)
  local wait = (ahead - self.lead * self.rate) / self.rate
  if wait > 0 then ecm.sleep(wait) end
end

--- Close the device files. A transmitting SDR first lets its queued samples
--- go out (at most lead + 1 s) and then switches tx off.
function SDR:close()
  local ok, e = pcall(function()
    if self._keyed then
      if self._tx_start ~= nil then
        local left = (self._sent - (self:timestamp() - self._tx_start)) / self.rate
        if left > 0 then ecm.sleep(math.min(left, self.lead + 1.0)) end
      end
      self:tx(false)
    end
  end)
  for _, k in ipairs({ "_rx", "_tx" }) do
    if self[k] ~= nil then self[k]:close() end
    self[k] = nil
  end
  if not ok then error(e, 0) end
end

local ToneSource = node_class(Source)

--- A tone at `freq` Hz (complex: e^{jwt}; real: cos). Options: rate (48000),
--- amplitude (0.5), seconds (nil = endless), kind ("complex").
function M.tone(freq, ...)
  local o = args({ "rate", "amplitude", "seconds", "kind" }, ...)
  local t = setmetatable({}, ToneSource)
  t.freq = freq
  t.rate = o.rate or 48000.0
  t.amplitude = o.amplitude or 0.5
  t.kind = o.kind or COMPLEX
  t.left = o.seconds and math.floor(o.seconds * t.rate) or nil
  t.phase = 0.0
  return t
end

function ToneSource:read(n)
  if self.left ~= nil then
    if self.left <= 0 then return nil end
    n = math.min(n, self.left)
    self.left = self.left - n
  end
  local data
  data, self.phase = native.tone(self.freq, self.rate, n, self.amplitude, self.phase, self.kind)
  return samples(self.kind, data, self.rate)
end

local FileSource = node_class(Source)

--- IQ from a .cf32 / .cs16 / .sigmf-data file; rate and frequency from the
--- .sigmf-meta beside it when present. Options: fmt, rate, loop (start again at the end).
function M.file_source(path, ...)
  local o = args({ "fmt", "rate", "loop" }, ...)
  local meta
  local ok, text = pcall(read_file, meta_path(path))
  if ok then
    local ok2, m = pcall(native.sigmf_parse, text)
    if ok2 then meta = m end
  end
  local fmt, rate = o.fmt, o.rate
  if fmt == nil then
    if meta ~= nil then
      fmt = meta.datatype
    elseif lower_ends(path, ".cs16", ".ci16") then
      fmt = "cs16"
    else
      fmt = "cf32"
    end
  end
  if rate == nil then
    if meta == nil then radio_error(path .. " has no .sigmf-meta: pass rate", 2) end
    rate = meta.rate
  end
  local s = setmetatable({}, FileSource)
  s.meta = meta
  s.freq = meta and meta.freq or 0.0
  s.rate = rate
  s.kind = COMPLEX
  s.format = fmt
  s.repeat_ = o.loop and true or false
  s.data = native.decode(fmt, read_file(path))
  s.pos = 0
  return s
end

local function read_chunk(self, n, width, kind)
  if self.pos >= #self.data then
    if not self.repeat_ or #self.data == 0 then return nil end
    self.pos = 0
  end
  local chunk = self.data:sub(self.pos + 1, self.pos + width * n)
  self.pos = self.pos + #chunk
  return samples(kind, chunk, self.rate)
end

function FileSource:read(n) return read_chunk(self, n, 8, COMPLEX) end

local WavSource = node_class(Source)

--- Audio from a WAV file (or headerless 16-bit PCM at raw_rate). Options: loop, raw_rate (8000).
function M.wav_source(path, ...)
  local o = args({ "loop", "raw_rate" }, ...)
  local s = setmetatable({}, WavSource)
  local rate
  rate, s.data = native.read_audio_file(path, read_file(path), o.raw_rate or 8000)
  s.rate = rate
  s.kind = REAL
  s.repeat_ = o.loop and true or false
  s.pos = 0
  return s
end

function WavSource:read(n) return read_chunk(self, n, 4, REAL) end

local FramesSource = node_class(Source)

--- Packet bodies to modulate (strings), one per read. `rate` is the sample
--- rate the modulator will produce.
function M.frames_source(frames, ...)
  local o = args({ "rate" }, ...)
  local s = setmetatable({}, FramesSource)
  s.frames = {}
  for i, f in ipairs(frames) do s.frames[i] = tostring(f) end
  s.rate = o.rate or 48000.0
  s.kind = FRAMES
  return s
end

function FramesSource:read(n)
  if #self.frames == 0 then return nil end
  return samples(FRAMES, { table.remove(self.frames, 1) }, self.rate)
end

-- ---------------------------------------------------------------- sinks

local Sink = node_class()
M.Sink = Sink
Sink.__tostring = function() return "<sink>" end

--- Check the incoming stream; may return blocks to append (e.g. a resampler).
function Sink:prepare(kind, rate) return {} end
function Sink:write(s) error("not implemented", 2) end
function Sink:close() end

local SpeakerSink = node_class(Sink)

--- Play real audio on the Speaker (/dev/audio). Resamples when needed.
--- Options: side, volume, rate.
function M.speaker(...)
  local o = args({ "side", "volume", "rate" }, ...)
  local suffix = o.side == nil and "" or ("." .. o.side)
  return setmetatable({ device = "/dev/audio" .. suffix, ctl = "/dev/audioctl" .. suffix,
    volume = o.volume, want_rate = o.rate }, SpeakerSink)
end

function SpeakerSink:prepare(kind, rate)
  if kind ~= REAL then radio_error("speaker() plays real audio, got " .. kind .. " (add a demodulator)") end
  local extra = {}
  local target = self.want_rate
  if target == nil then
    if rate >= 8000 and rate <= 48000 and math.abs(rate - math.floor(rate + 0.5)) < 1e-6 then
      target = rate
    else
      target = 24000
    end
  end
  if math.abs(target - rate) > 0.5 then extra[#extra + 1] = M.resample(target) end
  self.rate = math.floor(target + 0.5)
  local f, e = open_file(self.device, "wb", true)
  if not f then radio_error(e) end
  self._pcm = f
  write_control(self.ctl, "rate " .. self.rate)
  write_control(self.ctl, "bits 16")
  write_control(self.ctl, "channels 1")
  if self.volume ~= nil then write_control(self.ctl, string.format("volume %d", self.volume)) end
  return extra
end

function SpeakerSink:write(s)
  if s:count() > 0 then self._pcm:write(native.pcm16(s.data)) end
end

function SpeakerSink:close()
  if self._pcm ~= nil then
    self._pcm:close()
    self._pcm = nil
  end
end

local WavSink = node_class(Sink)

function M.wav_sink(path)
  return setmetatable({ path = path, chunks = {}, rate = 0 }, WavSink)
end

function WavSink:prepare(kind, rate)
  if kind ~= REAL then radio_error("wav_sink() records real audio, got " .. kind) end
  self.rate = math.floor(rate + 0.5)
  return {}
end

function WavSink:write(s) self.chunks[#self.chunks + 1] = native.pcm16(s.data) end

function WavSink:close()
  local pcm = table.concat(self.chunks)
  local f, e = open_file(self.path, "wb")
  if not f then error(e, 0) end
  f:write(native.wav_header(self.rate, math.floor(#pcm / 2)))
  f:write(pcm)
  f:close()
end

local IqSink = node_class(Sink)

--- Record complex samples (.cf32 / .cs16) plus a SigMF .sigmf-meta.
--- Options: fmt, freq, description.
function M.iq_sink(path, ...)
  local o = args({ "fmt", "freq", "description" }, ...)
  local fmt = o.fmt
  if fmt == nil then fmt = lower_ends(path, ".cs16", ".ci16") and "cs16" or "cf32" end
  return setmetatable({ path = path, format = fmt, freq = native.parse_freq(o.freq or 0.0),
    description = o.description or "", meta = true, rate = 0.0 }, IqSink)
end

function IqSink:prepare(kind, rate)
  if kind ~= COMPLEX then radio_error("iq_sink() records complex samples, got " .. kind) end
  self.rate = rate
  local f, e = open_file(self.path, "wb")
  if not f then radio_error(e) end
  self._f = f
  return {}
end

function IqSink:write(s) self._f:write(native.encode(self.format, s.data)) end

function IqSink:close()
  if self._f ~= nil then
    self._f:close()
    self._f = nil
  end
  if self.meta then
    local text = native.sigmf_meta(self.format, self.rate, self.freq, self.description, nil, "ecm radio.lua")
    local f, e = open_file(meta_path(self.path), "wb")
    if not f then error(e, 0) end
    f:write(text)
    f:close()
  end
end

local Collect = node_class(Sink)
M.Collect = Collect

--- Keeps everything it receives: sink:samples() (one Samples) or sink.frames.
function M.collect()
  return setmetatable({ parts = {}, frames = {}, rate = 0.0 }, Collect)
end

function Collect:prepare(kind, rate)
  self.kind, self.rate = kind, rate
  return {}
end

function Collect:write(s)
  if s.kind == FRAMES then
    for _, f in ipairs(s.data) do self.frames[#self.frames + 1] = f end
  else
    self.parts[#self.parts + 1] = s.data
  end
end

function Collect:samples()
  return samples(self.kind, table.concat(self.parts), self.rate)
end

local FramesSink = node_class(Sink)

--- Decoded packets: printed (AX.25 when it parses) and/or passed to
--- `callback`. echo defaults to true.
function M.frames(...)
  local o = args({ "callback", "echo" }, ...)
  local echo = o.echo
  if echo == nil then echo = true end
  return setmetatable({ callback = o.callback, echo = echo, frames = {} }, FramesSink)
end

function FramesSink:prepare(kind, rate)
  if kind ~= FRAMES then radio_error("frames() takes decoded packets; put a modem (e.g. afsk1200()) in front") end
  return {}
end

function FramesSink:write(s)
  for _, f in ipairs(s.data) do
    self.frames[#self.frames + 1] = f
    if self.echo then
      local d = native.ax25_decode(f)
      if d ~= nil then
        print(string.format("%s>%s: %s", d.src, d.dest, d.text))
      else
        print(string.format("frame %d bytes: %q", #f, f))
      end
    end
    if self.callback ~= nil then self.callback(f) end
  end
end

-- ---------------------------------------------------------------- flowgraph

local Flowgraph = class()
M.FlowgraphClass = Flowgraph

--- Runs `source .. blocks... .. sink`. Without a sink, output is collected
--- (see fg.sink:samples() / fg.sink.frames).
function M.Flowgraph(p, block_size)
  local items = {}
  for i, x in ipairs(items_of(p)) do items[i] = x end
  if #items == 0 or not is_a(items[1], Source) then
    radio_error("a flowgraph starts with a source (an SDR, tone(), file_source(), ...)", 2)
  end
  local fg = setmetatable({}, Flowgraph)
  fg.source = table.remove(items, 1)
  local last = items[#items]
  if last ~= nil and (is_a(last, Sink) or is_a(last, SDR)) then
    fg.sink = table.remove(items)
  else
    fg.sink = M.collect()
  end
  for _, b in ipairs(items) do
    if not is_a(b, Block) then radio_error(tostring(b) .. " can't be in the middle of a flowgraph", 2) end
  end
  fg.blocks = items
  fg.block_size = block_size or 4096
  local probe = chain(fg.blocks, fg.source.kind, fg.source.rate)
  local extra = fg.sink:prepare(probe.kind, probe.rate)
  if #extra > 0 then
    probe:close()
    for _, b in ipairs(extra) do fg.blocks[#fg.blocks + 1] = b end
    probe = chain(fg.blocks, fg.source.kind, fg.source.rate)
  end
  fg.chain = probe
  fg.consumed = 0
  fg.last_read = 0
  fg.idle = false
  fg._stop = false
  return fg
end

function Flowgraph:describe() return self.chain.description end

--- Process one buffer; false when the source is exhausted.
--- fg.last_read is how many samples the source gave (0 = nothing yet).
function Flowgraph:step()
  local s = self.source:read(self.block_size)
  if s == nil then return false end
  self.last_read = s:count()
  self.consumed = self.consumed + self.last_read
  local out = self.chain:process(s)
  if out:count() > 0 then self.sink:write(out) end
  return true
end

--- End run() after the current buffer (e.g. from a frames() callback).
function Flowgraph:stop() self._stop = true end

--- run{seconds=, samples=, timeout=} (or run(seconds, samples, timeout)):
--- until the source ends, that much input, stop(), or `timeout` seconds in a
--- row without input (10 by default; fg.idle is then true; false = forever).
function Flowgraph:run(...)
  local o = args({ "seconds", "samples", "timeout" }, ...)
  local limit = o.samples
  if o.seconds ~= nil then limit = math.floor(o.seconds * self.source.rate) end
  local timeout = o.timeout
  if timeout == nil then timeout = 10.0 end
  self._stop = false
  self.idle = false
  local ok, e = pcall(function()
    local quiet_since
    while not self._stop and (limit == nil or self.consumed < limit) do
      if not self:step() then break end
      if self.last_read > 0 then
        quiet_since = nil
      elseif timeout then
        local now = ecm.now()
        if quiet_since == nil then
          quiet_since = now
        elseif now - quiet_since >= timeout then
          self.idle = true
          break
        end
        ecm.sleep(0.01)
      end
    end
  end)
  self:close()
  if not ok then error(e, 0) end
  return self.sink
end

--- Close the chain, the sink (an SDR sink stops transmitting) and the source
--- (an SDR source's device file; the next read reopens it).
function Flowgraph:close()
  self.chain:close()
  local ok, e = pcall(function() self.sink:close() end)
  if self.source ~= self.sink then self.source:close() end
  if not ok then error(e, 0) end
end

-- ---------------------------------------------------------------- helpers

--- An AX.25 UI frame body (for afsk1200()).
function M.ax25(dest, src, info, pid)
  return native.ax25_encode(dest, src, info, pid or 0xF0)
end

--- {src, dest, digis, pid, info, text} or nil.
function M.parse_ax25(frame) return native.ax25_decode(frame) end

--- { {freq, power_db, snr_db}, ... } of signals in a complex block.
--- Options: center (0), nfft (1024), threshold (15).
function M.find_signals(s, ...)
  local o = args({ "center", "nfft", "threshold" }, ...)
  return native.find_signals(s.data, s.rate, o.center or 0.0, o.nfft or 1024, o.threshold or 15.0)
end

function M.parse_freq(value) return native.parse_freq(value) end

-- ---------------------------------------------------------------- framed PHY mode

M.BROADCAST = 0xFFFFFFFF

local Framed = class()
M.FramedClass = Framed

--- The framed radio called `name`: "" (first SDR), "sdr_0" / 0, or an
--- attachment name. Options: frames_path, ctl_path, tx_path. See Python's
--- radio.Framed and docs/radio/SDR_PROGRAMS.md for the records.
function M.framed(name, opts)
  if type(name) == "table" then name, opts = "", name end
  opts = opts or {}
  local f = setmetatable({}, Framed)
  local fp, cp = opts.frames_path, opts.ctl_path
  if fp == nil or cp == nil then
    local a, b = native.resolve_framed(tostring(name or ""))
    fp = fp or a
    cp = cp or b
  end
  f.frames_path, f.ctl_path, f.tx_path = fp, cp, opts.tx_path or fp
  f._pending, f._tag = {}, 0
  local ok, e = pcall(read_status, cp)
  if not ok then radio_error(string.format("no framed radio at %s: %s", cp, tostring(e)), 2) end
  return f
end

function Framed:control(command) write_control(self.ctl_path, command) end

function Framed:status() return read_status(self.ctl_path) end

--- Switch to framed mode and set it up: configure{phy=, role=, channel=,
--- bandwidth=, power=, address=, promiscuous=} (nil leaves a setting).
function Framed:configure(o)
  o = o or {}
  self:control("mode framed")
  if o.phy ~= nil then self:control("phy " .. o.phy) end
  if o.role ~= nil then self:control("role " .. o.role) end
  if o.channel ~= nil then self:control(string.format("channel %d", o.channel)) end
  if o.bandwidth ~= nil then self:control(string.format("phy_bw %d", math.floor(native.parse_freq(o.bandwidth) + 0.5))) end
  if o.power ~= nil then self:control("power " .. tostring(o.power)) end
  if o.address ~= nil then self:control(string.format("address %08x", o.address)) end
  if o.promiscuous ~= nil then self:control("promisc " .. (o.promiscuous and 1 or 0)) end
  return self:status()
end

--- Back to IQ mode (an SDR).
function Framed:iq_mode() self:control("mode iq") end

function Framed:address() return tonumber(tostring(self:status().address), 16) end

function Framed:slot() return math.floor(tonumber(self:status().slot) or 0) end

function Framed:_write(record)
  if self._tx == nil then
    local f, e = open_file(self.tx_path, "wb", true)
    if not f then radio_error(e) end
    self._tx = f
  end
  self._tx:write(record)
end

--- Send one frame: send(payload, {dst=, ack=, slot=, cqi=, subchannel=, wait=}).
--- With wait (default) returns its status table ("sent", or with ack "acked"/"failed").
function Framed:send(payload, o)
  o = o or {}
  self._tag = (self._tag + 1) % 0x10000
  local tag = self._tag
  self:_write(native.framed_tx((o.dst or M.BROADCAST) % 0x100000000, tostring(payload), o.ack and true or false,
    o.slot or 0, o.cqi or 0, o.subchannel or 0, tag))
  if o.wait == false then return nil end
  local deadline = ecm.now() + 3.0
  while ecm.now() < deadline do
    for i, r in ipairs(self._pending) do
      if r.type == "status" and r.tag == tag then return table.remove(self._pending, i) end
    end
    self:_fill()
  end
  return nil
end

--- Repeat `payload` every period_ms on the airtime clock (0 stops beacon `index`).
function Framed:beacon(payload, period_ms, index)
  self:_write(native.framed_beacon(index or 0, period_ms or 100, tostring(payload)))
end

function Framed:_fill()
  if self._rx == nil then
    local f, e = open_file(self.frames_path, "rb", true)
    if not f then radio_error(e) end
    self._rx = f
  end
  local data = self._rx:read(65536) or ""
  local recs = native.framed_parse(data)
  for _, r in ipairs(recs) do self._pending[#self._pending + 1] = r end
  return #recs > 0
end

--- Received frames (rx tables) within `timeout` seconds (1 by default), at most `count`.
function Framed:recv(timeout, count)
  timeout = timeout or 1.0
  local out = {}
  local deadline = ecm.now() + timeout
  while count == nil or #out < count do
    while #self._pending > 0 and (count == nil or #out < count) do
      local r = table.remove(self._pending, 1)
      if r.type == "rx" then out[#out + 1] = r end
    end
    if count ~= nil and #out >= count then break end
    if ecm.now() >= deadline then break end
    self:_fill()
  end
  return out
end

function Framed:close()
  for _, k in ipairs({ "_rx", "_tx" }) do
    if self[k] ~= nil then self[k]:close() end
    self[k] = nil
  end
end

return M
