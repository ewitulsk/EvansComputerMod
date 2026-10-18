--[[ Speaker and microphone audio: /dev/audio of a Speaker and /dev/mic of a
Microphone next to the computer (or the ePhone's own).
The Lua counterpart of Python's `audio` module (same functions).

    local spk = audio.open()            -- the first speaker; audio.open("left") for a side
    spk:tone(440, 0.25)                 -- a quarter second of A4 (square wave)
    spk:notes("C4 E4 G4 C5", 0.15)      -- note names, each 0.15 s; "-" is a rest
    spk:set_format(8000, 8, 1)          -- rate, bits, channels
    spk:write(pcm)                      -- raw PCM in the current format (waits for room)
    print(spk:status().buffered_ms)     -- {rate=8000, ..., buffered_ms=40, underruns=0}
    spk:close()

The speaker plays samples at its rate; writes wait while more than `latency`
ms of audio is queued, so a loop that just writes stays in time with the
music. Formats: rate 8000-48000 Hz, 8-bit unsigned or 16-bit signed
little-endian, mono or stereo (mixed down: a speaker is one point source).

    local mic = audio.open_mic()        -- the default microphone; audio.open_mic("top") for a side
    mic:set_format(16000)               -- 16-bit mono by default
    local pcm = mic:record(2.0)         -- two seconds of 16-bit PCM (waits while it records)
    print(audio.peak_db(pcm))           -- loudest sample, dBFS (-99 = silence)
    audio.save_wav("/home/memo.wav", pcm, 16000)
    print(mic:status().overruns)
    mic:close()

A microphone hears players talking and speakers playing around it, quieter
with distance. Capture starts with the first read and runs on its own clock:
a read waits until the audio it asks for has been captured, and a program
that reads less than once a second loses audio (an overrun). Gain 0-100:
50 is the capsule's own level, every 25 is 10 dB, 0 mutes.

With no side given, open() and open_mic() use the default devices chosen in
Settings > Sound (/etc/sound.conf), else the first speaker / microphone.
]]

local M = {}
M.TONE_RATE = 8000
M.SOUND_CONF = "/etc/sound.conf"
M.MIXER = "/dev/mixer"
M.MAX_APP_PERCENT = 150
M.ROLES = { "media", "voice", "phone" }
local NOTES = { C = -9, D = -7, E = -5, F = -4, G = -2, A = 0, B = 2 }

--- "A4" -> 440, "C#5", "Eb3" ...
function M.note_freq(name)
  local letter = name:sub(1, 1):upper()
  local semis = NOTES[letter]
  if semis == nil then error("not a note name: " .. tostring(name), 2) end
  local i = 2
  while i <= #name and (name:sub(i, i) == "#" or name:sub(i, i) == "b") do
    semis = semis + (name:sub(i, i) == "#" and 1 or -1)
    i = i + 1
  end
  local octave = 4
  if i <= #name then octave = tonumber(name:sub(i)) or error("not a note name: " .. name, 2) end
  return 440.0 * 2 ^ ((semis + 12 * (octave - 4)) / 12.0)
end

local Speaker = {}
Speaker.__index = Speaker

local function open_unbuffered(path, mode)
  local f, e = io.open(path, mode)
  if not f then error(e, 3) end
  f:setvbuf("no")
  return f
end

--- The side Settings > Sound chose for kind "output" (speaker) or "input" (microphone), or nil.
function M.default_device(kind)
  local f = io.open(M.SOUND_CONF, "rb")
  if not f then return nil end
  local text = f:read("*a") or ""
  f:close()
  for line in text:gmatch("[^\n]+") do
    local k, v = line:match("^%s*(%S+)%s+(%S+)")
    if k == kind then return v end
  end
  return nil
end

-- The device suffix for an explicit side, else the default if its device exists, else "".
local function side_for(side, kind, prefix)
  if side ~= nil then return "." .. side end
  local d = M.default_device(kind)
  if d then
    local f = io.open(prefix .. "." .. d, "rb")
    if f then f:close() return "." .. d end
  end
  return ""
end

local function ctl_status(path)
  local ctl = open_unbuffered(path, "rb")
  local text = ctl:read("*a") or ""
  ctl:close()
  local out = {}
  for line in text:gmatch("[^\n]+") do
    local k, v = line:match("^%s*(%S+)%s+(%S+)%s*$")
    if k and tonumber(v) then out[k] = tonumber(v) end
  end
  return out
end

--- Loudest sample of 16-bit little-endian PCM (a string), in dBFS (-99 for silence).
function M.peak_db(pcm)
  local peak = 0
  for i = 1, #pcm - 1, 2 do
    local lo, hi = pcm:byte(i, i + 1)
    local v = lo + hi * 256
    if v >= 32768 then v = v - 65536 end
    if v < 0 then v = -v end
    if v > peak then peak = v end
  end
  if peak == 0 then return -99 end
  local db = 20 * math.log(peak / 32768.0) / math.log(10)
  db = db >= 0 and math.floor(db + 0.5) or -math.floor(-db + 0.5)
  return math.max(-99, db)
end

local function le(v, n)
  local t = {}
  for i = 1, n do
    t[i] = string.char(v % 256)
    v = math.floor(v / 256)
  end
  return table.concat(t)
end

--- A 44-byte RIFF/WAVE header for data_len bytes of PCM.
function M.wav_header(rate, bits, channels, data_len)
  local block = math.floor(channels * bits / 8)
  return "RIFF" .. le(36 + data_len, 4) .. "WAVEfmt " .. le(16, 4) .. le(1, 2) .. le(channels, 2)
    .. le(rate, 4) .. le(rate * block, 4) .. le(block, 2) .. le(bits, 2) .. "data" .. le(data_len, 4)
end

--- Write PCM (a string) as a WAV file.
function M.save_wav(path, pcm, rate, bits, channels)
  bits, channels = bits or 16, channels or 1
  local f, e = io.open(path, "wb")
  if not f then error(e, 2) end
  f:write(M.wav_header(rate, bits, channels, #pcm) .. pcm)
  f:close()
end

local function new(side)
  local suffix = side_for(side, "output", "/dev/audio")
  local s = setmetatable({}, Speaker)
  s.device = "/dev/audio" .. suffix
  s._ctl_path = "/dev/audioctl" .. suffix
  s._pcm = open_unbuffered(s.device, "wb")
  s._format = nil
  return s
end

--- Send one /dev/audioctl command, e.g. "volume 50" or "flush".
function Speaker:control(command)
  local ctl = open_unbuffered(self._ctl_path, "wb")
  ctl:write(command .. "\n")
  ctl:close()
end

function Speaker:set_format(rate, bits, channels)
  rate, bits, channels = rate or 48000, bits or 16, channels or 1
  self:control(string.format("rate %d", rate))
  self:control(string.format("bits %d", bits))
  self:control(string.format("channels %d", channels))
  self._format = rate .. "/" .. bits .. "/" .. channels
end

function Speaker:set_volume(volume)
  self:control(string.format("volume %d", volume))
end

--- Who this stream is for: "media" (default), "voice" (a call) or "phone" (a cellular
--- call, a ringtone). While a stream carries sound, every stream with a lower role is
--- ducked (media < voice < phone).
function Speaker:set_role(role)
  local ok = false
  for _, r in ipairs(M.ROLES) do if r == role then ok = true end end
  if not ok then error("role is media, voice or phone, not " .. tostring(role), 2) end
  self:control("role " .. role)
end

--- true while a louder, higher-role stream lowers this one.
function Speaker:ducked()
  return self:status().ducked == 1
end

--- Drop whatever is queued.
function Speaker:flush() self:control("flush") end

function Speaker:status()
  return ctl_status(self._ctl_path)
end

--- Raw PCM bytes (a string) in the current format.
function Speaker:write(pcm)
  self._pcm:write(pcm)
end

--- Play a tone (8 kHz, 8-bit). wave: "square" (default), "triangle" or "saw".
function Speaker:tone(freq, seconds, volume, wave)
  volume, wave = volume or 0.5, wave or "square"
  if self._format ~= M.TONE_RATE .. "/8/1" then
    self:set_format(M.TONE_RATE, 8, 1)
  end
  local n = math.floor(M.TONE_RATE * seconds)
  if freq <= 0 then
    self:write(string.rep(string.char(128), n))
    return
  end
  local amp = math.floor(math.max(0, math.min(1, volume)) * 100)
  local period = M.TONE_RATE / freq
  local out, char, abs = {}, string.char, math.abs
  for i = 0, n - 1 do
    local ph = (i % period) / period
    local v
    if wave == "triangle" then
      v = 1 - 4 * abs(ph - 0.5)
    elseif wave == "saw" then
      v = 2 * ph - 1
    else
      v = ph < 0.5 and 1 or -1
    end
    -- int() in Python truncates toward zero
    local x = v * amp
    x = x >= 0 and math.floor(x) or math.ceil(x)
    out[i + 1] = char(128 + x)
  end
  self:write(table.concat(out))
end

--- Play note names separated by spaces ("C4 E4 G4 -"); "-" is a rest.
function Speaker:notes(melody, seconds, volume, wave)
  seconds = seconds or 0.2
  for name in melody:gmatch("%S+") do
    local f = name == "-" and 0 or M.note_freq(name)
    self:tone(f, seconds * 0.9, volume, wave)
    self:tone(0, seconds * 0.1)
  end
end

function Speaker:close()
  self._pcm:close()
end

--- The speaker attached as `side` ("left", "top", ...), or the default / first one.
function M.open(side)
  return new(side)
end

--- The same as open(): a speaker stream of its own.
function M.open_speaker(side)
  return new(side)
end

-- A peak (0-32767) as a meter fill 0-100 on a dB scale from -60 dBFS.
local function meter(peak)
  if peak <= 0 then return 0 end
  local db = 20 * math.log(peak / 32767) / math.log(10)
  return math.max(0, math.min(100, math.floor((db + 60) / 0.6 + 0.5)))
end

local function is_master(target)
  return target == "master" or target:sub(1, 7) == "master."
end

--- /dev/mixer's text as a list of strip tables ({kind = "output"|"app"|"input", ...}).
function M.parse_mixer(text)
  local out = {}
  for line in text:gmatch("[^\n]+") do
    local words = {}
    for w in line:gmatch("%S+") do words[#words + 1] = w end
    local kind, name = words[1], words[2]
    if name and (kind == "output" or kind == "app" or kind == "input") then
      local kv = {}
      for i = 3, #words do
        local k, v = words[i]:match("^([^=]+)=(.*)$")
        if k then kv[k] = v end
      end
      local function num(k, d) return math.floor(tonumber(kv[k]) or d or 0) end
      local function side(k)
        local v = kv[k]
        if v == nil or v == "" or v == "-" then return nil end
        return v
      end
      if kind == "output" then
        local peak = num("peak")
        out[#out + 1] = { kind = "output", side = name, target = "master." .. name, volume = num("volume"),
          mute = num("mute") == 1, peak = peak, meter = meter(peak), label = (kv.label or ""):gsub("_", " ") }
      elseif kind == "app" then
        local peak = num("peak")
        out[#out + 1] = { kind = "app", app = name, target = name, volume = num("volume"), mute = num("mute") == 1,
          route = side("route"), on = side("on"), peak = peak, meter = meter(peak), ducked = num("ducked") == 1,
          role = kv.role or "media", streams = num("streams") }
      else
        local level = num("level", -99)
        out[#out + 1] = { kind = "input", side = name, target = name, gain = num("gain"), level = level,
          meter = math.floor(math.max(0, math.min(60, level + 60)) * 100 / 60) }
      end
    end
  end
  return out
end

local Mixer = {}
Mixer.__index = Mixer

--- Every strip as a table with kind "output", "app" or "input".
function Mixer:strips()
  local f = open_unbuffered(self.device, "rb")
  local text = f:read("*a") or ""
  f:close()
  return M.parse_mixer(text)
end

function Mixer:_send(cmd)
  local f, e = io.open(self.device, "wb")
  if not f then error(e, 3) end
  f:setvbuf("no")
  local ok, werr = f:write(cmd .. "\n")
  f:close()
  if not ok then
    if tostring(werr):lower():find("permi") then
      error("changing the Volume Mixer needs the mixer permission", 3)
    end
    error("the Volume Mixer refused '" .. cmd .. "' (" .. tostring(werr) .. ")", 3)
  end
end

--- An app's volume 0-150 %, or a speaker's ("master", "master.left") 0-100 %.
function Mixer:set(target, percent)
  percent = math.floor(percent)
  local top = is_master(target) and 100 or M.MAX_APP_PERCENT
  if percent < 0 or percent > top then
    error((top == 100 and "a speaker" or "an app") .. "'s volume is 0-" .. top, 2)
  end
  self:_send(string.format("set %s %d", target, percent))
end

--- Mute (or with on = false unmute) an app or a speaker.
function Mixer:mute(target, on)
  self:_send(string.format("mute %s %s", target, on == false and "off" or "on"))
end

--- Play an app on the speaker attached as `side`; nil (or "-") = the default speaker.
function Mixer:route(app, side)
  if is_master(app) then error("route moves an app; a speaker can't be routed", 2) end
  if side == nil or side == "" or side == "default" then side = "-" end
  self:_send(string.format("route %s %s", app, side))
end

--- A microphone's gain 0-100 by its side.
function Mixer:gain(input, n)
  n = math.floor(n)
  if n < 0 or n > 100 then error("gain is 0-100", 2) end
  self:_send(string.format("gain %s %d", input, n))
end

--- Forget an app's volume, mute and route.
function Mixer:reset(app)
  self:_send("reset " .. app)
end

--- The Volume Mixer: m:strips(), m:set(target, percent), m:mute(target, on),
--- m:route(app, side), m:gain(input, n), m:reset(app).
function M.mixer()
  return setmetatable({ device = M.MIXER }, Mixer)
end

local Microphone = {}
Microphone.__index = Microphone

--- Send one /dev/micctl command, e.g. "gain 60" or "flush".
function Microphone:control(command)
  local ctl = open_unbuffered(self._ctl_path, "wb")
  ctl:write(command .. "\n")
  ctl:close()
end

function Microphone:set_format(rate, bits, channels)
  rate, bits, channels = rate or 16000, bits or 16, channels or 1
  self:control(string.format("rate %d", rate))
  self:control(string.format("bits %d", bits))
  self:control(string.format("channels %d", channels))
  self.rate, self.bits, self.channels = rate, bits, channels
end

--- Input volume 0-100 (50 = the capsule's own level, 0 = mute).
function Microphone:set_gain(gain)
  self:control(string.format("gain %d", gain))
end

--- Drop what was captured and not read yet.
function Microphone:flush() self:control("flush") end

function Microphone:status()
  return ctl_status(self._ctl_path)
end

--- Peak of the last read, dBFS (-99 before the first).
function Microphone:level()
  return self:status().level_db or -99
end

--- Raw PCM in the current format: waits until nbytes (whole frames, at most half a second) are captured.
function Microphone:read(nbytes)
  return self._pcm:read(nbytes) or ""
end

--- `seconds` of audio as PCM (a string) in the current format (waits while it records).
function Microphone:record(seconds)
  local frame = math.floor(self.bits / 8) * self.channels
  local want = math.floor(seconds * self.rate + 0.5) * frame
  local chunk = math.max(frame, math.floor(self.rate / 10) * frame)
  local parts, got = {}, 0
  while got < want do
    local s = self._pcm:read(math.min(chunk, want - got))
    if not s or #s == 0 then break end
    parts[#parts + 1] = s
    got = got + #s
  end
  return table.concat(parts)
end

function Microphone:close()
  self._pcm:close()
end

--- The microphone attached as `side` ("top", ...; the ePhone's is "microphone"), or the default / first one.
function M.open_mic(side)
  local suffix = side_for(side, "input", "/dev/mic")
  local m = setmetatable({}, Microphone)
  m.device = "/dev/mic" .. suffix
  m._ctl_path = "/dev/micctl" .. suffix
  m._pcm = open_unbuffered(m.device, "rb")
  local st = m:status()
  m.rate, m.bits, m.channels = st.rate or 48000, st.bits or 16, st.channels or 1
  return m
end

return M
