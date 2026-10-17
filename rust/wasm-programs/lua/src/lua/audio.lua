--[[ Speaker audio: the /dev/audio device of a Speaker next to the computer.
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
]]

local M = {}
M.TONE_RATE = 8000
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

local function new(side)
  local suffix = side == nil and "" or ("." .. side)
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

--- Drop whatever is queued.
function Speaker:flush() self:control("flush") end

function Speaker:status()
  local ctl = open_unbuffered(self._ctl_path, "rb")
  local text = ctl:read("*a") or ""
  ctl:close()
  local out = {}
  for line in text:gmatch("[^\n]+") do
    local k, v = line:match("^%s*(%S+)%s+(%S+)%s*$")
    if k and tonumber(v) then out[k] = tonumber(v) end
  end
  return out
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

--- The speaker attached as `side` ("left", "top", ...), or the first one.
function M.open(side)
  return new(side)
end

return M
