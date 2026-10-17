--[[ Cards: bank chips, magnetic stripes, NFC sectors, SIMs, the readers that
talk to them and the printer that paints them (docs/cards/CARDS.md).
The Lua counterpart of Python's `cards` module (same functions).

    local reader = cards.Reader.find()      -- the first Card Reader/Writer (block or bay module)
    local card = reader:card()              -- the card in its slot, or nil
    local id = card:identity()              -- {kind=, uid=, applets=, ...}

    local t1, t2, t3 = card.stripe:read()   -- plain text, anyone can read or clone it
    card.stripe:write("ANN EXAMPLE", "12345678", "")

    local key = cards.random_hex(32)        -- bank chip: keyed, counted; the key never leaves
    card.bank:write("acme", "4000123412341234", key, "1234", true)
    card.bank:verify_pin("1234")
    local ctr, tag = card.bank:sign(nonce_hex, 1500)
    cards.verify_tag(key, nonce_hex, ctr, 1500, tag)   -- what the issuer checks

    card.nfc:auth(1, cards.FACTORY_KEY)     -- NFC sectors by contact
    card.nfc:write(1, "48656c6c6f")
    reader:eject()

    local nfc = cards.Nfc.find()            -- an NFC Reader (block or module)
    nfc:field(true)
    for _, seen in ipairs(nfc:poll()) do    -- { {uid=, kinds=}, ... }
      local s = nfc:session(seen.uid, 1, cards.FACTORY_KEY)
      print(s:read())                       -- 64 bytes, hex
    end

    local printer = cards.Printer.find()
    local d = cards.Design.new("bank")      -- or cards.Design.load("acme.card")
    d.palette = { {255, 255, 255}, {0, 0, 0}, {200, 30, 30} }
    d:fill("front", 2)
    d:text("front", 1, 1, "ACME BANK", 1)   -- 3x5 font: digits, A-Z, space, '-', '.'
    d.fields.name = "Ann Example"
    d.stripe = { "ANN EXAMPLE", "12345678", "" }
    printer:print(d)
    d:save("acme.card")

Faces are palette-indexed bitmaps: 32x20 texels for bank / nfc / hybrid, 8x13 (portrait)
for a SIM, at most 16 colours, index 0 = the first (x, y and indices start at
0 as in Python; d.palette is a Lua list, so colour index i is d.palette[i + 1]).
Bytes are hex strings everywhere. mix, tag, verify_tag and pin_hash are the
reference implementations of docs/cards/NFC_MIX.md. A peripheral's error is a
Lua error (pcall).
]]

local peripheral = require("peripheral")
local bit = bit32

local M = {}

-- ---------------------------------------------------------------- bytes and hashes

local function hexstr(s)
  return (s:gsub(".", function(c) return string.format("%02x", c:byte()) end))
end
M.hex = hexstr

local function unhex(text, n, what)
  what = what or "value"
  if type(text) ~= "string" or #text % 2 ~= 0 or text:find("[^0-9a-fA-F]") then
    error(what .. " is not hex: " .. tostring(text), 3)
  end
  local out = text:gsub("..", function(h) return string.char(tonumber(h, 16)) end)
  if n and #out ~= n then
    error(string.format("%s is %d bytes (%d hex digits), got %d", what, n, 2 * n, #out), 3)
  end
  return out
end
M.unhex = unhex

--- `n` random bytes as hex.
function M.random_hex(n)
  local t = {}
  for i = 1, n do t[i] = string.format("%02x", math.random(0, 255)) end
  return table.concat(t)
end

local function u32be(s, i)
  local a, b, c, d = s:byte(i, i + 3)
  return ((a * 256 + b) * 256 + c) * 256 + d
end

local function u32le(s, i)
  local a, b, c, d = s:byte(i, i + 3)
  return ((d * 256 + c) * 256 + b) * 256 + a
end

local function be32(v)
  return string.char(bit.rshift(v, 24), bit.band(bit.rshift(v, 16), 255), bit.band(bit.rshift(v, 8), 255), bit.band(v, 255))
end

local function le32(v)
  return string.char(bit.band(v, 255), bit.band(bit.rshift(v, 8), 255), bit.band(bit.rshift(v, 16), 255), bit.rshift(v, 24))
end

local function be64(v)
  -- A non-negative number below 2^53 (every counter and amount in play), or
  -- 16 hex digits for a full u64.
  if type(v) == "string" then return unhex(v, 8, "a 64-bit value") end
  local hi = math.floor(v / 4294967296)
  local lo = v - hi * 4294967296
  return be32(hi) .. be32(lo)
end

local SHA_K = {
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
  0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
  0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
  0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
  0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
  0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
  0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
  0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
  0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
}
local SHA_H = { 0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19 }

local band, bxor, bnot, rrot, lrot, rshift = bit.band, bit.bxor, bit.bnot, bit.rrotate, bit.lrotate, bit.rshift

--- SHA-256 of a byte string -> 32 bytes. Pure Lua: the in-game interpreter has no hash library.
local function sha256(data)
  local len = #data
  local msg = data .. "\128" .. string.rep("\0", (55 - len) % 64) .. be64(len * 8)
  local h = { table.unpack(SHA_H) }
  local w = {}
  for start = 1, #msg, 64 do
    for i = 0, 15 do w[i] = u32be(msg, start + 4 * i) end
    for i = 16, 63 do
      local s0 = bxor(rrot(w[i - 15], 7), rrot(w[i - 15], 18), rshift(w[i - 15], 3))
      local s1 = bxor(rrot(w[i - 2], 17), rrot(w[i - 2], 19), rshift(w[i - 2], 10))
      w[i] = (w[i - 16] + s0 + w[i - 7] + s1) % 4294967296
    end
    local a, b, c, d, e, f, g, hh = h[1], h[2], h[3], h[4], h[5], h[6], h[7], h[8]
    for i = 0, 63 do
      local t1 = (hh + bxor(rrot(e, 6), rrot(e, 11), rrot(e, 25)) + bxor(band(e, f), band(bnot(e), g)) + SHA_K[i + 1] + w[i]) % 4294967296
      local t2 = (bxor(rrot(a, 2), rrot(a, 13), rrot(a, 22)) + bxor(band(a, b), band(a, c), band(b, c))) % 4294967296
      hh, g, f, e, d, c, b, a = g, f, e, (d + t1) % 4294967296, c, b, a, (t1 + t2) % 4294967296
    end
    local v = { a, b, c, d, e, f, g, hh }
    for i = 1, 8 do h[i] = (h[i] + v[i]) % 4294967296 end
  end
  local out = {}
  for i = 1, 8 do out[i] = be32(h[i]) end
  return table.concat(out)
end

local function hmac_sha256(key, msg)
  if #key > 64 then key = sha256(key) end
  key = key .. string.rep("\0", 64 - #key)
  local ipad = key:gsub(".", function(c) return string.char(bxor(c:byte(), 0x36)) end)
  local opad = key:gsub(".", function(c) return string.char(bxor(c:byte(), 0x5C)) end)
  return sha256(opad .. sha256(ipad .. msg))
end

--- SHA-256 of hex bytes, as hex.
function M.sha256(hex_text) return hexstr(sha256(unhex(hex_text))) end

--- HMAC-SHA256 of hex bytes under a hex key, as hex.
function M.hmac_sha256(key_hex, msg_hex) return hexstr(hmac_sha256(unhex(key_hex), unhex(msg_hex))) end

-- ---------------------------------------------------------------- the schemes (NFC_MIX.md)

M.FACTORY_KEY = "45434d2d4e46432d464143544f525930"   -- ASCII "ECM-NFC-FACTORY0": every sector of a blank
M.SECTORS = 8
M.SECTOR_BYTES = 64

local function mix(key, x)
  local k = { u32le(key, 1), u32le(key, 5), u32le(key, 9), u32le(key, 13) }
  local v0, v1, v2, v3 = u32le(x, 1), u32le(x, 5), u32le(x, 9), u32le(x, 13)
  for r = 0, 7 do
    v0 = (v0 + k[r % 4 + 1]) % 4294967296
    v1 = bxor(v1, lrot(v0, 7))
    v2 = (v2 + v1) % 4294967296
    v3 = bxor(v3, lrot(v2, 13))
    v0 = (v0 + v3) % 4294967296
    v1 = (lrot(v1, 17) + k[(r + 1) % 4 + 1]) % 4294967296
    v2 = bxor(v2, lrot(v0, 5))
    v3 = (bxor(v3, r) + k[(r + 2) % 4 + 1]) % 4294967296
  end
  return le32(v0) .. le32(v1) .. le32(v2) .. le32(v3)
end

--- Sector Lattice Mix(key[16], x[16]) -> 16 bytes, all hex.
function M.mix(key_hex, x_hex)
  return hexstr(mix(unhex(key_hex, 16, "key"), unhex(x_hex, 16, "x")))
end

--- The card's answer M1 = Mix(S_i, R1 || R2).
function M.auth_m1(key_hex, r1_hex, r2_hex)
  return M.mix(key_hex, hexstr(unhex(r1_hex, 8, "R1") .. unhex(r2_hex, 8, "R2")))
end

--- The reader's answer M2 = Mix(S_i, R2 || R1).
function M.auth_m2(key_hex, r1_hex, r2_hex)
  return M.mix(key_hex, hexstr(unhex(r2_hex, 8, "R2") .. unhex(r1_hex, 8, "R1")))
end

--- Bank TAG = HMAC-SHA256(K, "ECM-BANK-TAG" || NONCE || CTR || AMOUNT)[0..16), hex.
function M.tag(key_hex, nonce_hex, ctr, amount)
  local msg = "ECM-BANK-TAG" .. unhex(nonce_hex, 8, "nonce") .. be64(ctr) .. be64(amount or 0)
  return hexstr(hmac_sha256(unhex(key_hex, 32, "key"), msg):sub(1, 16))
end

--- Whether a card's tag for (nonce, ctr, amount) is right under the issuer's copy of K.
function M.verify_tag(key_hex, nonce_hex, ctr, amount, tag_hex)
  local want = M.tag(key_hex, nonce_hex, ctr, amount)
  local got = tostring(tag_hex):lower()
  if #got ~= #want then return false end
  local d = 0
  for i = 1, #want do d = bit.bor(d, bxor(want:byte(i), got:byte(i))) end
  return d == 0
end

--- SHA-256("ECM-BANK-PIN" || UID[8] || PIN ASCII), hex.
function M.pin_hash(uid_hex, pin)
  return hexstr(sha256("ECM-BANK-PIN" .. unhex(uid_hex, 8, "uid") .. tostring(pin)))
end

-- ---------------------------------------------------------------- NFC air frames

M.CMD_POLL, M.CMD_SELECT, M.CMD_AUTH, M.CMD_AUTH2, M.CMD_READ, M.CMD_WRITE, M.CMD_SETKEY, M.CMD_ERROR = 1, 2, 3, 4, 5, 6, 7, 0xFF
M.REPLY_BIT = 0x80
M.ERR_BAD_AUTH, M.ERR_NO_SESSION, M.ERR_BAD_SECTOR, M.ERR_NOT_SELECTED, M.ERR_MALFORMED = 1, 2, 3, 4, 5
local CMD_NAMES = { [1] = "POLL", [2] = "SELECT", [3] = "AUTH", [4] = "AUTH2", [5] = "READ", [6] = "WRITE", [7] = "SETKEY", [0xFF] = "ERROR" }
local REASONS = { [1] = "bad auth", [2] = "no session", [3] = "bad sector", [4] = "not selected", [5] = "malformed" }

--- An NF-106 air frame cmd(1) len(1) body, as hex.
function M.frame_encode(cmd, body_hex)
  local body = unhex(body_hex or "", nil, "body")
  if #body > 255 then error("a frame body is at most 255 bytes", 2) end
  return hexstr(string.char(band(cmd, 0xFF), #body) .. body)
end

--- cmd, body_hex of an air frame; an error when malformed.
function M.frame_decode(frame_hex)
  local b = unhex(frame_hex, nil, "frame")
  if #b < 2 then error("frame shorter than 2 bytes", 2) end
  if #b ~= 2 + b:byte(2) then error("frame length does not match its len byte", 2) end
  return b:byte(1), hexstr(b:sub(3))
end

--- POLL, SELECT, ... for a request or reply cmd byte.
function M.frame_name(cmd)
  if cmd == M.CMD_ERROR then return "ERROR" end
  return CMD_NAMES[band(cmd, 0x7F)] or "?"
end

function M.error_reason(reason) return REASONS[reason] or "?" end

-- ---------------------------------------------------------------- designs

M.KINDS = { "bank", "nfc", "hybrid", "sim" }

-- The 3x5 font: five rows per glyph, each a 3-bit value with the left texel in
-- bit 2 (shared verbatim with ecm-cards and cards.py).
M.FONT = {
  ["0"] = { 7, 5, 5, 5, 7 }, ["1"] = { 2, 6, 2, 2, 7 }, ["2"] = { 7, 1, 7, 4, 7 }, ["3"] = { 7, 1, 7, 1, 7 },
  ["4"] = { 5, 5, 7, 1, 1 }, ["5"] = { 7, 4, 7, 1, 7 }, ["6"] = { 7, 4, 7, 5, 7 }, ["7"] = { 7, 1, 1, 1, 1 },
  ["8"] = { 7, 5, 7, 5, 7 }, ["9"] = { 7, 5, 7, 1, 7 },
  A = { 2, 5, 7, 5, 5 }, B = { 6, 5, 6, 5, 6 }, C = { 7, 4, 4, 4, 7 }, D = { 6, 5, 5, 5, 6 },
  E = { 7, 4, 7, 4, 7 }, F = { 7, 4, 7, 4, 4 }, G = { 7, 4, 5, 5, 7 }, H = { 5, 5, 7, 5, 5 },
  I = { 7, 2, 2, 2, 7 }, J = { 1, 1, 1, 5, 7 }, K = { 5, 5, 6, 5, 5 }, L = { 4, 4, 4, 4, 7 },
  M = { 5, 7, 7, 5, 5 }, N = { 6, 5, 5, 5, 5 }, O = { 7, 5, 5, 5, 7 }, P = { 7, 5, 7, 4, 4 },
  Q = { 7, 5, 5, 7, 1 }, R = { 7, 5, 6, 5, 5 }, S = { 7, 4, 7, 1, 7 }, T = { 7, 2, 2, 2, 2 },
  U = { 5, 5, 5, 5, 7 }, V = { 5, 5, 5, 5, 2 }, W = { 5, 5, 7, 7, 5 }, X = { 5, 5, 2, 5, 5 },
  Y = { 5, 5, 2, 2, 2 }, Z = { 7, 1, 2, 4, 7 },
  [" "] = { 0, 0, 0, 0, 0 }, ["-"] = { 0, 0, 7, 0, 0 }, ["."] = { 0, 0, 0, 0, 2 },
}
M.GLYPH_ADVANCE = 4

--- width, height in texels of a card kind's faces.
function M.face_size(kind)
  if kind == "bank" or kind == "nfc" or kind == "hybrid" then return 32, 20 end
  if kind == "sim" then return 8, 13 end
  error("unknown card kind '" .. tostring(kind) .. "' (bank, nfc, hybrid, sim)", 2)
end

local B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"

local function b64encode(data)
  local out = {}
  for i = 1, #data, 3 do
    local a, b, c = data:byte(i, i + 2)
    local n = (c and 3) or (b and 2) or 1
    local v = a * 65536 + (b or 0) * 256 + (c or 0)
    local q1, q2, q3, q4 = rshift(v, 18), band(rshift(v, 12), 63), band(rshift(v, 6), 63), band(v, 63)
    out[#out + 1] = B64:sub(q1 + 1, q1 + 1) .. B64:sub(q2 + 1, q2 + 1)
      .. (n > 1 and B64:sub(q3 + 1, q3 + 1) or "=") .. (n > 2 and B64:sub(q4 + 1, q4 + 1) or "=")
  end
  return table.concat(out)
end

local B64_INDEX = {}
for i = 1, 64 do B64_INDEX[B64:sub(i, i)] = i - 1 end
B64_INDEX["-"], B64_INDEX["_"] = 62, 63

local function b64decode(text)
  local out, acc, bits = {}, 0, 0
  for c in tostring(text):gmatch(".") do
    if c ~= "=" and c ~= "\n" and c ~= "\r" and c ~= " " then
      local v = B64_INDEX[c]
      if v == nil then error("not base64", 3) end
      acc = acc * 64 + v
      bits = bits + 6
      if bits >= 8 then
        bits = bits - 8
        local byte = math.floor(acc / 2 ^ bits)
        out[#out + 1] = string.char(byte)
        acc = acc - byte * 2 ^ bits
      end
    end
  end
  return table.concat(out)
end

local function read_text(path)
  local f = io.open(path, "rb")
  if f then
    local s = f:read("*a")
    f:close()
    return s
  end
  local ok, shell = pcall(require, "shell")
  local text = ok and type(shell) == "table" and shell.read_file(path) or nil
  if text == nil then error("can't read " .. path, 3) end
  return text
end

local function write_text(path, text)
  local f = io.open(path, "wb")
  if f then
    f:write(text)
    f:close()
    return
  end
  local ok, shell = pcall(require, "shell")
  if not (ok and type(shell) == "table" and shell.write_file and shell.write_file(path, text)) then
    error("can't write " .. path, 3)
  end
end

local Design = {}
Design.__index = Design
Design.__tostring = function(d)
  return string.format("<card design %s %dx%d, %d colours, %d fields%s>", d.kind, d.width, d.height,
    #d.palette, (function() local n = 0 for _ in pairs(d.fields) do n = n + 1 end return n end)(),
    d.stripe and ", stripe" or "")
end
M.Design = Design

--- A design: kind, palette ({ {r, g, b}, ... }, at most 16), front / back
--- (byte strings of packed 4-bit indices), fields (table of strings, at most 8)
--- and stripe ({t1, t2, t3} or nil).
local function new_design(kind, palette, front, back, fields, stripe)
  local w, h = M.face_size(kind)
  local n = math.floor((w * h + 1) / 2)
  local d = setmetatable({}, Design)
  d.kind, d.width, d.height = kind, w, h
  d.palette = {}
  for i, c in ipairs(palette or { { 255, 255, 255 } }) do d.palette[i] = { c[1], c[2], c[3] } end
  front = front or ""
  back = back or ""
  d.front = front .. string.rep("\0", n - #front)
  d.back = back .. string.rep("\0", n - #back)
  d.fields = {}
  for k, v in pairs(fields or {}) do d.fields[k] = v end
  d.stripe = stripe and { stripe[1] or "", stripe[2] or "", stripe[3] or "" } or nil
  return d
end

--- A blank design of `kind` (every texel index 0, a one-entry white palette).
function Design.new(kind) return new_design(kind) end

local function face_key(face)
  if face == "front" or face == 0 then return "front" end
  if face == "back" or face == 1 then return "back" end
  error("face is 'front' or 'back'", 3)
end

--- The palette index at (x, y) (0 outside the face).
function Design:pixel(face, x, y)
  if x < 0 or y < 0 or x >= self.width or y >= self.height then return 0 end
  local i = y * self.width + x
  local b = self[face_key(face)]:byte(math.floor(i / 2) + 1)
  if i % 2 == 0 then return rshift(b, 4) end
  return band(b, 15)
end

--- Set the palette index at (x, y); outside the face nothing happens.
function Design:set_pixel(face, x, y, idx)
  if x < 0 or y < 0 or x >= self.width or y >= self.height then return end
  local key = face_key(face)
  local data = self[key]
  local i = y * self.width + x
  local pos = math.floor(i / 2) + 1
  local b = data:byte(pos)
  idx = band(math.floor(idx), 15)
  if i % 2 == 0 then b = band(b, 0x0F) + idx * 16 else b = band(b, 0xF0) + idx end
  self[key] = data:sub(1, pos - 1) .. string.char(b) .. data:sub(pos + 1)
end

function Design:fill(face, idx)
  local key = face_key(face)
  self[key] = string.rep(string.char(band(math.floor(idx), 15) * 0x11), #self[key])
end

--- Draw `s` with the 3x5 font, top-left at (x, y), in palette index `idx`
--- (only set pixels are painted). Returns the x after the text.
function Design:text(face, x, y, s, idx)
  idx = idx == nil and 1 or idx
  local cx = x
  for c in tostring(s):upper():gmatch(".") do
    local rows = M.FONT[c]
    if rows then
      for dy = 0, 4 do
        for dx = 0, 2 do
          if band(rows[dy + 1], rshift(4, dx)) ~= 0 then self:set_pixel(face, cx + dx, y + dy, idx) end
        end
      end
    end
    cx = cx + M.GLYPH_ADVANCE
  end
  return cx
end

--- The palette index of a colour, adding it when there is room; else the nearest.
function Design:color(r, g, b)
  for i, c in ipairs(self.palette) do
    if c[1] == r and c[2] == g and c[3] == b then return i - 1 end
  end
  if #self.palette < 16 then
    self.palette[#self.palette + 1] = { r, g, b }
    return #self.palette - 1
  end
  local best, bd
  for i, c in ipairs(self.palette) do
    local d = (c[1] - r) ^ 2 + (c[2] - g) ^ 2 + (c[3] - b) ^ 2
    if bd == nil or d < bd then best, bd = i - 1, d end
  end
  return best
end

local function fields_copy(fields)
  local out = {}
  for k, v in pairs(fields) do out[k] = v end
  return out
end

--- The design as the card_printer peripheral takes it: {palette, front = hex, back = hex, fields, stripe?}.
function Design:to_map()
  local palette = {}
  for i, c in ipairs(self.palette) do palette[i] = { math.floor(c[1]), math.floor(c[2]), math.floor(c[3]) } end
  local m = { palette = palette, front = hexstr(self.front), back = hexstr(self.back), fields = fields_copy(self.fields) }
  if self.stripe then
    m.stripe = { t1 = tostring(self.stripe[1] or ""), t2 = tostring(self.stripe[2] or ""), t3 = tostring(self.stripe[3] or "") }
  end
  return m
end

--- The `.card` file text (keys in the order ecm-cards and cards.py write them).
function Design:to_json()
  local dumps = require("_ui").json_dumps
  local palette = {}
  for i, c in ipairs(self.palette) do palette[i] = { math.floor(c[1]), math.floor(c[2]), math.floor(c[3]) } end
  local parts = {
    '"kind":' .. dumps(self.kind),
    '"palette":' .. (#palette > 0 and dumps(palette) or "[]"),
    '"front":"' .. b64encode(self.front) .. '"',
    '"back":"' .. b64encode(self.back) .. '"',
    '"fields":' .. dumps(fields_copy(self.fields)),
  }
  if self.stripe then
    parts[#parts + 1] = '"stripe":{"t1":' .. dumps(tostring(self.stripe[1] or "")) .. ',"t2":' .. dumps(tostring(self.stripe[2] or ""))
      .. ',"t3":' .. dumps(tostring(self.stripe[3] or "")) .. "}"
  end
  return "{" .. table.concat(parts, ",") .. "}"
end

function Design.from_json(text)
  local j = require("_ui").json_loads(text)
  if type(j) ~= "table" or type(j.kind) ~= "string" then error("a .card file is a JSON object with a kind", 2) end
  local w, h = M.face_size(j.kind)
  local n = math.floor((w * h + 1) / 2)
  local palette = {}
  for i, c in ipairs(j.palette or {}) do
    if type(c) ~= "table" or #c ~= 3 then error("a palette entry is [r, g, b]", 2) end
    for _, v in ipairs(c) do
      if type(v) ~= "number" or v < 0 or v > 255 then error("a palette entry is [r, g, b]", 2) end
    end
    palette[i] = { math.floor(c[1]), math.floor(c[2]), math.floor(c[3]) }
  end
  local function face(key)
    local v = j[key]
    if v == nil then return string.rep("\0", n) end
    local b = b64decode(v)
    if #b > n then error(string.format('"%s" holds more than %dx%d texels', key, w, h), 2) end
    return b
  end
  local stripe
  if type(j.stripe) == "table" then
    stripe = { tostring(j.stripe.t1 or ""), tostring(j.stripe.t2 or ""), tostring(j.stripe.t3 or "") }
  end
  return new_design(j.kind, #palette > 0 and palette or nil, face("front"), face("back"), j.fields or {}, stripe)
end

--- Read a `.card` file.
function Design.load(path) return Design.from_json(read_text(path)) end

function Design:save(path) write_text(path, self:to_json()) end

-- ---------------------------------------------------------------- readers

local function design_map(design)
  if getmetatable(design) == Design then return design:to_map() end
  return design
end

local function pad_sector(data_hex)
  local b = unhex(data_hex, nil, "sector data")
  if #b > M.SECTOR_BYTES then error("a sector holds " .. M.SECTOR_BYTES .. " bytes", 3) end
  return hexstr(b .. string.rep("\0", M.SECTOR_BYTES - #b))
end

-- The magnetic stripe of the card in a reader: plain text, no secret.
local Stripe = {}
Stripe.__index = Stripe

--- t1, t2, t3, or nil without a stripe.
function Stripe:read()
  local r = self._p.call("read_stripe")
  if r == nil then return nil end
  return r.t1 or "", r.t2 or "", r.t3 or ""
end

function Stripe:corrupted()
  local r = self._p.call("read_stripe")
  return r ~= nil and r.corrupted == true
end

--- Write the tracks (64 / 32 / 96 printable characters) and clear `corrupted`.
function Stripe:write(t1, t2, t3)
  return self._p.call("write_stripe", tostring(t1), tostring(t2 or ""), tostring(t3 or ""))
end

-- The bank chip of the card in a reader (contact only).
local Bank = {}
Bank.__index = Bank

--- Write a blank chip once: issuer [a-z0-9_-]{1,24}, PAN 8-19 digits, 32-byte
--- key (hex, keep it: the card never gives it back), PIN 4-8 digits.
function Bank:write(issuer, pan, key_hex, pin, pin_required)
  unhex(key_hex, 32, "key")
  return self._p.call("write_bank", tostring(issuer), tostring(pan), tostring(key_hex):lower(), tostring(pin or ""), pin_required == true)
end

--- Open a PIN session (until the card leaves); false on a wrong PIN (three failures lock the chip).
function Bank:verify_pin(pin)
  local r = self._p.call("verify_pin", tostring(pin))
  if type(r) == "table" then return r.ok == true end
  return r == true
end

--- ctr, tag_hex for an 8-byte nonce; the chip increments ctr first.
function Bank:sign(nonce_hex, amount)
  unhex(nonce_hex, 8, "nonce")
  local r = self._p.call("sign", tostring(nonce_hex):lower(), math.floor(amount or 0))
  return r.ctr, tostring(r.tag)
end

function Bank:blank() return self._p.call("bank_blank") == true end

-- The NFC sectors of the card in a reader, by contact.
local NfcApplet = {}
NfcApplet.__index = NfcApplet

function NfcApplet:auth(sector, key_hex)
  unhex(key_hex, 16, "sector key")
  return self._p.call("nfc_auth", math.floor(sector), tostring(key_hex):lower())
end

--- 64 bytes of an authenticated sector, hex.
function NfcApplet:read(sector) return self._p.call("nfc_read", math.floor(sector)) end

--- Write up to 64 bytes (hex, zero-padded).
function NfcApplet:write(sector, data_hex) return self._p.call("nfc_write", math.floor(sector), pad_sector(data_hex)) end

function NfcApplet:set_key(sector, key_hex)
  unhex(key_hex, 16, "sector key")
  return self._p.call("nfc_set_key", math.floor(sector), tostring(key_hex):lower())
end

-- The SIM applet of the card in a reader (what `cardtool sim` uses).
local Sim = {}
Sim.__index = Sim

--- Write a blank SIM once (16-byte key, hex); returns the reader's result.
function Sim:write(imsi, carrier, key_hex)
  unhex(key_hex, 16, "key")
  return self._p.call("write", tostring(imsi), tostring(carrier), tostring(key_hex):lower())
end

--- {res=, ks=} for a 16-byte challenge.
function Sim:respond(challenge_hex)
  unhex(challenge_hex, 16, "challenge")
  return self._p.call("respond", tostring(challenge_hex):lower())
end

function Sim:is_blank() return self._p.call("is_blank") == true end

-- The card in a reader's slot: stripe, bank, nfc and sim are its applets.
local Card = {}
Card.__index = Card
Card.__tostring = function(c) return "<card in " .. c.reader.peripheral.name .. ">" end

local function new_card(reader)
  local p = reader.peripheral
  return setmetatable({
    reader = reader,
    stripe = setmetatable({ _p = p }, Stripe),
    bank = setmetatable({ _p = p }, Bank),
    nfc = setmetatable({ _p = p }, NfcApplet),
    sim = setmetatable({ _p = p }, Sim),
  }, Card)
end

--- {kind, uid?, applets, print, bank?: {issuer, pan, locked, ctr, blank}, nfc?: {sectors}, sim?: {imsi, carrier, blank}}
function Card:identity() return self.reader.peripheral.call("identity") end

function Card:kind()
  local id = self:identity()
  return id.kind or id.type
end

--- Set the public printed fields (name, number, issuer, label, ...).
function Card:write_fields(fields) return self.reader.peripheral.call("write_fields", fields) end

local function find(cls, name)
  local p
  if name == nil then
    p = peripheral.find(cls.TYPE)
  else
    p = peripheral.wrap(name)
    if p ~= nil and p.type ~= cls.TYPE then
      error(string.format("'%s' is a %s, not a %s", name, p.type, cls.TYPE), 3)
    end
  end
  if p == nil then return nil end
  return setmetatable({ peripheral = p, name = p.name }, cls)
end

local function find_all(cls)
  local out = {}
  for i, p in ipairs(peripheral.find_all(cls.TYPE)) do out[i] = setmetatable({ peripheral = p, name = p.name }, cls) end
  return out
end

-- A Card Reader/Writer (block) or Card Reader Module (bay): peripheral type card_reader.
local Reader = { TYPE = "card_reader" }
Reader.__index = Reader
Reader.__tostring = function(r) return "<card reader '" .. r.name .. "'>" end
M.Reader = Reader

--- The reader attached as `name`, or the first one; nil without.
function Reader.find(name) return find(Reader, name) end
function Reader.find_all() return find_all(Reader) end

function Reader:present() return self.peripheral.call("present") == true end

--- {present, released, auto_eject, state, applets}
function Reader:status() return self.peripheral.call("status") end

--- The card in the slot, or nil.
function Reader:card()
  if not self:present() then return nil end
  return new_card(self)
end

--- Release the card (Auto Eject pushes it into an adjacent inventory): true
--- when it was ejected, false when it waits to be taken.
function Reader:eject() return self.peripheral.call("eject") == true end

--- The next card_inserted / card_removed event of this reader as
--- event, kind, applets / event, or nil after `timeout` seconds (30 by default).
function Reader:wait(timeout)
  timeout = timeout == nil and 30 or timeout
  local waited = 0
  while timeout == nil or waited < timeout do
    local ev = { peripheral.pull_event(nil, 0.5) }
    waited = waited + 0.5
    if ev[1] ~= nil and ev[2] == self.name and tostring(ev[1]):sub(1, 5) == "card_" then
      return ev[1], table.unpack(ev, 3)
    end
  end
  return nil
end

-- An authenticated sector on a card in an NFC reader's field.
local Session = {}
Session.__index = Session

--- 64 bytes, hex.
function Session:read() return self.nfc:read(self.sector) end
function Session:write(data_hex) return self.nfc:write(self.sector, data_hex) end
function Session:set_key(key_hex) return self.nfc:set_key(self.sector, key_hex) end

-- An NFC Reader (block) or NFC module (bay): peripheral type nfc_reader.
local Nfc = { TYPE = "nfc_reader" }
Nfc.__index = Nfc
Nfc.__tostring = function(n) return "<nfc reader '" .. n.name .. "'>" end
M.Nfc = Nfc

function Nfc.find(name) return find(Nfc, name) end
function Nfc.find_all() return find_all(Nfc) end

--- Energise the field (nfc:field(true)), or ask whether it is on.
function Nfc:field(on)
  if on == nil then return self.peripheral.call("field") == true end
  return self.peripheral.call("set_field", on == true)
end

function Nfc:power(dbm) return self.peripheral.call("set_power", math.floor(dbm)) end

--- { {uid=, kinds=}, ... } heard in the field (the field is switched on if needed).
function Nfc:poll()
  if not self:field() then self:field(true) end
  return self.peripheral.call("poll") or {}
end

function Nfc:select(uid) return self.peripheral.call("select", tostring(uid):lower()) end

function Nfc:auth(sector, key_hex)
  unhex(key_hex, 16, "sector key")
  return self.peripheral.call("auth", math.floor(sector), tostring(key_hex):lower())
end

function Nfc:read(sector) return self.peripheral.call("read", math.floor(sector)) end

function Nfc:write(sector, data_hex) return self.peripheral.call("write", math.floor(sector), pad_sector(data_hex)) end

function Nfc:set_key(sector, key_hex)
  unhex(key_hex, 16, "sector key")
  return self.peripheral.call("set_key", math.floor(sector), tostring(key_hex):lower())
end

--- select(uid) + auth(sector, key): a session with read / write / set_key.
function Nfc:session(uid, sector, key_hex)
  self:select(uid)
  local r = self:auth(sector, key_hex)
  if r == false then error(string.format("auth failed for sector %d of %s", math.floor(sector), tostring(uid)), 2) end
  return setmetatable({ nfc = self, uid = tostring(uid):lower(), sector = math.floor(sector) }, Session)
end

--- Send an arbitrary air frame (hex) and return the reply (hex).
function Nfc:raw(frame_hex) return self.peripheral.call("raw", hexstr(unhex(frame_hex, nil, "frame"))) end

function Nfc:rssi() return self.peripheral.call("rssi") end

-- A Card Printer: peripheral type card_printer. It only paints.
local Printer = { TYPE = "card_printer" }
Printer.__index = Printer
Printer.__tostring = function(p) return "<card printer '" .. p.name .. "'>" end
M.Printer = Printer

function Printer.find(name) return find(Printer, name) end
function Printer.find_all() return find_all(Printer) end

function Printer:present() return self.peripheral.call("present") == true end

--- {present, kind, printing, progress, dyes = {white = 3, ...}}
function Printer:status() return self.peripheral.call("status") end

--- The dyes a design takes (one per palette colour used), without printing.
function Printer:cost(design) return self.peripheral.call("cost", design_map(design)) end

--- Print a design (or its to_map()) on the card in the printer; the card is
--- released when done (print_done / print_failed events).
function Printer:print(design) return self.peripheral.call("print", design_map(design)) end

function Printer:eject() return self.peripheral.call("eject") == true end

return M
