-- phone: the phone's number, calls and texts, through telephonyd (man phone; docs/TELEPHONY.md).
-- The same names as Python's phone (phone.py).
--
--   local phone = require("phone")
--
--   print(phone.number())                    -- "+12145550142", or nil without a SIM
--   local id = phone.sms_send("+12145550143", "On my way")
--   print(phone.sms_status(id))              -- "sending" / "sent" / "delivered" / "failed"
--   for _, m in ipairs(phone.sms_inbox()) do print(m.from, m.text) end
--   phone.mms_send({"+12145550143", "+12145550144"}, "Look", {"/home/Pictures/view.png"})
--
--   local call = phone.dial("611")
--   call:send_tones("1")  call:play("/home/hello.wav")  local path = call:record(5)  call:hangup()
--
--   phone.on_sms(function(sender, text) print(sender, text) end)
--   phone.on_mms(function(sender, group, text, files) print(sender, #files) end)
--   phone.on_call(function(call) call:answer() call:hangup() end)
--   phone.run()                              -- or phone.poll(seconds) for what is waiting
--
--   phone.contacts()                         -- { {name=, numbers={ {label=, number=} }, emails=, note=}, ... }
--   phone.add_contact("Alex", "+12145550143")
--
-- A script needs the permissions an app would: phone (everything), sms (texts
-- only), contacts (the address book); texts and calls a non-system app starts are
-- confirmed by the player. Errors are Lua errors starting with "phone: ".

local ecm = require("_ecm")

local phone = {}

phone.PORT = 7720
phone.SMS_PORT = 7726
phone.CONTACTS = "/home/.contacts/contacts.vcf"

-- The transport (tests replace it).
phone._net = nil
local function N()
  if phone._net == nil then phone._net = require("net") end
  return phone._net
end

local function fail(msg) error("phone: " .. msg, 0) end

-- A Lua error raised by this module (the counterpart of Python's PhoneError).
phone.PhoneError = "phone"

local function enc(s)
  s = tostring(s)
  if s == "" then return "-" end
  if s == "-" then return "%2D" end
  local out = {}
  for i = 1, #s do
    local b = s:byte(i)
    local c = string.char(b)
    if b <= 32 or b >= 127 or c == "%" or c == "=" or c == "," then
      out[#out + 1] = string.format("%%%02X", b)
    else
      out[#out + 1] = c
    end
  end
  return table.concat(out)
end

local function dec(s)
  if s == nil or s == "-" then return "" end
  return (s:gsub("%%(%x%x)", function(h) return string.char(tonumber(h, 16)) end))
end

local function split(s, sep)
  local out = {}
  for part in (s .. sep):gmatch("(.-)" .. sep) do out[#out + 1] = part end
  return out
end

local function words(l)
  local w = {}
  for x in l:gmatch("%S+") do w[#w + 1] = x end
  return w
end

-- ------------------------------------------------------------- the control socket

local Conn = {}
Conn.__index = Conn

local function connect(port)
  local ok, fd = pcall(N().tcp_connect, "127.0.0.1", port)
  if not ok then fail("telephonyd is not running (or this script may not use port " .. port .. "): " .. tostring(fd)) end
  return setmetatable({ fd = fd, buf = "" }, Conn)
end

function Conn:line(wait)
  while not self.buf:find("\n", 1, true) do
    if wait == false and not N().tcp_ready(self.fd, 0) then return nil end
    local chunk = N().tcp_recv(self.fd, 4096)
    if chunk == nil or chunk == "" then fail("telephonyd closed the connection") end
    self.buf = self.buf .. chunk
  end
  local i = self.buf:find("\n", 1, true)
  local l = self.buf:sub(1, i - 1):gsub("\r$", "")
  self.buf = self.buf:sub(i + 1)
  return l
end

function Conn:request(req)
  N().tcp_send(self.fd, req .. "\n")
  local head = self:line()
  if head:sub(1, 3) == "err" then fail(head:sub(5) ~= "" and head:sub(5) or "refused") end
  local body = {}
  while true do
    local l = self:line()
    if l == "." then return body end
    if l:sub(1, 2) == ".." then l = l:sub(2) end
    body[#body + 1] = l
  end
end

function Conn:close() pcall(N().tcp_close, self.fd) end

local conns = {}

local function call(req, port)
  port = port or phone.PORT
  for attempt = 0, 1 do
    local c = conns[port]
    if c == nil then c = connect(port); conns[port] = c end
    local ok, r = pcall(c.request, c, req)
    if ok then return r end
    if type(r) == "string" and r:sub(1, 7) == "phone: " and not r:find("closed the connection", 1, true) then error(r, 0) end
    c:close()
    conns[port] = nil
    if attempt == 1 then fail("the connection to telephonyd failed") end
  end
end

local function first_id(body) return tonumber(words(body[1])[2]) end

-- ------------------------------------------------------------- status

-- registration, msisdn, domain, carrier, bars, volte, hd, voicemail, unread, missed.
function phone.status()
  local out = {}
  for _, l in ipairs(call("status", phone.SMS_PORT)) do
    local k, v = l:match("^(%S+) ?(.*)$")
    if k then out[k] = v end
  end
  return out
end

-- This phone's number (E.164), or nil.
function phone.number()
  local n = phone.status().msisdn
  if n == nil or n == "" or n == "-" then return nil end
  return n
end

-- ------------------------------------------------------------- texts

local function msg(l, thread)
  local w = words(l)
  local files = {}
  if w[9] and w[9] ~= "-" then
    for _, f in ipairs(split(w[9], ",")) do if f ~= "" then files[#files + 1] = dec(f) end end
  end
  return { id = tonumber(w[2]), incoming = w[3] == "in", from = dec(w[4]), time = tonumber(w[5]), status = dec(w[6]),
           kind = w[7], text = dec(w[8]), files = files, thread = thread }
end

-- { {key=, members=, unread=, time=, preview=}, ... } newest first.
function phone.threads()
  local out = {}
  for _, l in ipairs(call("threads", phone.SMS_PORT)) do
    local w = words(l)
    out[#out + 1] = { key = w[2], members = split(w[2], ","), unread = tonumber(w[3]), time = tonumber(w[4]), preview = dec(w[5]) }
  end
  return out
end

-- The last n messages of a thread (a number, or the group's numbers joined by commas).
function phone.messages(thread, n)
  local out = {}
  for _, l in ipairs(call("messages " .. thread .. " " .. (n or 50), phone.SMS_PORT)) do
    if l:sub(1, 4) == "msg " then out[#out + 1] = msg(l, thread) end
  end
  return out
end

-- Send a text; returns its id.
function phone.sms_send(to, text)
  return first_id(call("sms " .. enc(to) .. " " .. enc(text), phone.SMS_PORT))
end

-- A picture, voice note or group text: to is a number or a list (several = a group).
function phone.mms_send(to, text, files)
  if type(to) == "string" then to = { to } end
  local t = {}
  for i, x in ipairs(to) do t[i] = enc(x) end
  local f = {}
  for i, x in ipairs(files or {}) do f[i] = enc(x) end
  return first_id(call("mms " .. table.concat(t, ",") .. " " .. enc(text or "") .. " " .. (#f > 0 and table.concat(f, ",") or "-"), phone.SMS_PORT))
end

-- "sending", "sent", "delivered" or "failed" (nil: no such message).
function phone.sms_status(id)
  for _, t in ipairs(phone.threads()) do
    for _, m in ipairs(phone.messages(t.key, 200)) do
      if m.id == id then return m.status end
    end
  end
  return nil
end

-- Received texts and MMS, newest first.
function phone.sms_inbox(n)
  n = n or 50
  local out = {}
  for _, t in ipairs(phone.threads()) do
    for _, m in ipairs(phone.messages(t.key, n)) do
      if m.incoming then out[#out + 1] = m end
    end
  end
  table.sort(out, function(a, b) return a.time > b.time end)
  while #out > n do table.remove(out) end
  return out
end

function phone.mark_read(thread) call("read " .. thread, phone.SMS_PORT) end

-- ------------------------------------------------------------- calls

local Call = {}
Call.__index = Call
phone.Call = Call

local calls_by_id = {}

local function call_obj(id, number, incoming)
  local c = calls_by_id[id]
  if c == nil then
    c = setmetatable({ id = id, number = number or "", incoming = incoming or false, _tones = "" }, Call)
    calls_by_id[id] = c
  end
  return c
end

-- {state=, number=, hidden=, outgoing=, seconds=, muted=, hd=, codec=, loss=, merged=}, or nil once it ended.
function Call:info()
  for _, l in ipairs(call("calls")) do
    local w = words(l)
    if tonumber(w[2]) == self.id then
      return { state = w[3], number = dec(w[4]), hidden = w[5] == "1", outgoing = w[6] == "1", seconds = tonumber(w[7]),
               muted = w[8] == "1", hd = w[9] == "1", codec = dec(w[10]), loss = tonumber(w[11]), merged = w[12] == "1" }
    end
  end
  return nil
end

-- dialing, ringing (theirs), incoming, active, held, ended.
function Call:state()
  local i = self:info()
  return i and i.state or "ended"
end

-- Wait until the call is in one of states (default active/ended) or timeout seconds; returns the state.
function Call:wait(states, timeout)
  states = states or { "active", "ended" }
  local stop = ecm.now() + (timeout or 30)
  while true do
    local s = self:state()
    for _, x in ipairs(states) do if s == x then return s end end
    if ecm.now() >= stop then return s end
    phone.poll(0.2)
  end
end

function Call:answer() call("answer " .. self.id) end
function Call:reject() call("decline " .. self.id) end
function Call:hangup() call("hangup " .. self.id) end
function Call:hold(on) call("hold " .. self.id .. (on == false and " off" or " on")) end
function Call:mute(on) call("mute " .. self.id .. (on == false and " off" or " on")) end
-- Keypad tones (0-9 * # A-D) to the far end.
function Call:send_tones(digits) call("dtmf " .. self.id .. " " .. digits) end
-- Keypad tones the far end pressed since the last call.
function Call:tones()
  phone.poll()
  local t = self._tones
  self._tones = ""
  return t
end
-- Send a WAV file to the far end instead of the microphone.
function Call:play(wav) call("play " .. self.id .. " " .. enc(wav)) end
-- Record the far end for seconds into a WAV file; returns its path.
function Call:record(seconds, path)
  path = path or ("/home/Recordings/call-" .. self.id .. ".wav")
  call("record " .. self.id .. " " .. math.floor(seconds) .. " " .. enc(path))
  return path
end

-- Place a call (hide: withhold your number). Returns a Call.
function phone.dial(number, hide)
  local body = call("dial " .. enc(number) .. (hide and " hide" or ""))
  return call_obj(first_id(body), number, false)
end

-- The calls in progress.
function phone.calls()
  local out = {}
  for _, l in ipairs(call("calls")) do
    local w = words(l)
    out[#out + 1] = call_obj(tonumber(w[2]), dec(w[4]), w[6] == "0")
  end
  return out
end

function phone.speaker(on) call("speaker " .. (on == false and "off" or "on")) end

-- { {id=, dir= (in/out/missed), number=, start=, seconds=, reason=}, ... }.
function phone.call_log(n)
  local out = {}
  for _, l in ipairs(call("log " .. (n or 50))) do
    local w = words(l)
    out[#out + 1] = { id = tonumber(w[2]), dir = w[3], number = dec(w[4]), start = tonumber(w[5]), seconds = tonumber(w[6]), reason = dec(w[7]) }
  end
  return out
end

-- ------------------------------------------------------------- events

local handlers = { sms = {}, mms = {}, call = {} }
local watch = nil

local function open_watch()
  local port = (#handlers.call > 0) and phone.PORT or phone.SMS_PORT
  if watch ~= nil then
    if watch.port == port then return end
    watch.conn:close()
  end
  local c = connect(port)
  N().tcp_send(c.fd, "watch\n")
  if c:line():sub(1, 2) ~= "ok" then fail("telephonyd refused watch") end
  -- Texts and calls wake a sleeping phone for this script.
  pcall(N().set_wake, c.fd, true)
  watch = { port = port, conn = c }
end

-- fn(sender, text) for each new text (handled in run() / poll()).
function phone.on_sms(fn) table.insert(handlers.sms, fn); open_watch() end
-- fn(sender, group, text, files): group is the other members ({} for a 1:1); files under /home/Messages/.
function phone.on_mms(fn) table.insert(handlers.mms, fn); open_watch() end
-- fn(call) for each incoming call.
function phone.on_call(fn) table.insert(handlers.call, fn); open_watch() end

local function dispatch(l)
  local w = words(l)
  local name = w[1]
  local f = {}
  for i = 2, #w do
    local k, v = w[i]:match("^([^=]+)=(.*)$")
    if k then f[k] = dec(v) end
  end
  if name == "sms" then
    for _, fn in ipairs(handlers.sms) do fn(f.from or "", f.text or "") end
  elseif name == "mms" then
    local thread = f.thread or ""
    local group = {}
    if thread:find(",", 1, true) then
      local me = phone.number()
      for _, m in ipairs(split(thread, ",")) do
        if m ~= "" and m ~= f.from and m ~= me then group[#group + 1] = m end
      end
    end
    local files = {}
    for _, x in ipairs(split(f.files or "", ",")) do if x ~= "" then files[#files + 1] = x end end
    for _, fn in ipairs(handlers.mms) do fn(f.from or "", group, f.text or "", files) end
  elseif name == "incoming" then
    local c = call_obj(tonumber(f.call) or 0, f.from or "", true)
    for _, fn in ipairs(handlers.call) do fn(c) end
  elseif name == "dtmf" then
    local c = calls_by_id[tonumber(f.call) or 0]
    if c then c._tones = c._tones .. (f.digit or "") end
  end
end

-- Handle the events waiting (waiting up to timeout seconds for the first). Returns how many.
function phone.poll(timeout)
  timeout = timeout or 0
  if watch == nil then
    if timeout > 0 then ecm.sleep(timeout) end
    return 0
  end
  local c = watch.conn
  local n = 0
  local wait_ms = math.floor(timeout * 1000)
  while true do
    if not c.buf:find("\n", 1, true) and not N().tcp_ready(c.fd, wait_ms) then return n end
    wait_ms = 0
    local l = c:line()
    if l and l ~= "" then
      dispatch(l)
      n = n + 1
    end
  end
end

-- Handle events until the script is stopped.
function phone.run()
  if watch == nil then fail("nothing to wait for: register on_sms, on_mms or on_call first") end
  while true do phone.poll(1) end
end

-- ------------------------------------------------------------- contacts

local function vunescape(s)
  return (s:gsub("\\([nN,;\\])", function(c) if c == "n" or c == "N" then return "\n" end return c end))
end

local function vescape(s)
  return (s:gsub("[\\\n,;]", function(c) if c == "\n" then return "\\n" end return "\\" .. c end))
end

local function read_book()
  local f = io.open(phone.CONTACTS, "rb")
  if not f then return "" end
  local s = f:read("*a")
  f:close()
  return s or ""
end

-- The address book: { {name=, numbers={ {label=, number=} }, emails={}, note=, favourite=}, ... }.
function phone.contacts()
  local lines = {}
  for raw in (read_book():gsub("\r", "") .. "\n"):gmatch("(.-)\n") do
    local first = raw:sub(1, 1)
    if (first == " " or first == "\t") and #lines > 0 then
      lines[#lines] = lines[#lines] .. raw:sub(2)
    elseif raw:match("%S") then
      lines[#lines + 1] = raw
    end
  end
  local out, cur = {}, nil
  for _, l in ipairs(lines) do
    local key, value = l:match("^([^:]*):(.*)$")
    if key then
      local params = split(key, ";")
      local name = params[1]:upper()
      if name == "BEGIN" then
        cur = { name = "", numbers = {}, emails = {}, note = "", favourite = false }
      elseif cur ~= nil then
        if name == "FN" then cur.name = vunescape(value)
        elseif name == "TEL" then
          local label = ""
          for i = 2, #params do
            local t = params[i]:match("^[Tt][Yy][Pp][Ee]=(.*)$")
            if t then label = split(t, ",")[1]:lower() end
          end
          cur.numbers[#cur.numbers + 1] = { label = label, number = vunescape(value) }
        elseif name == "EMAIL" then cur.emails[#cur.emails + 1] = vunescape(value)
        elseif name == "NOTE" then cur.note = vunescape(value)
        elseif name == "X-ECM-FAVOURITE" then cur.favourite = value:match("^%s*1") ~= nil
        elseif name == "END" then out[#out + 1] = cur; cur = nil
        end
      end
    end
  end
  table.sort(out, function(a, b) return a.name:lower() < b.name:lower() end)
  return out
end

-- Add a contact to the address book (Contacts shows it).
function phone.add_contact(name, number, label)
  label = label or "cell"
  local w = words(name)
  local family = #w > 1 and table.remove(w) or ""
  local card = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:" .. vescape(name) .. "\r\nN:" .. vescape(family) .. ";" .. vescape(table.concat(w, " ")) ..
    ";;;\r\nTEL;TYPE=" .. label:upper() .. ":" .. vescape(number) .. "\r\nEND:VCARD\r\n"
  local text = read_book()
  if text ~= "" and text:sub(-1) ~= "\n" then text = text .. "\r\n" end
  ecm.mkdir((phone.CONTACTS:match("^(.*)/[^/]*$")) or ".")
  local f = io.open(phone.CONTACTS, "wb")
  if not f then fail("can't write the address book (does this script have the contacts permission?)") end
  f:write(text .. card)
  f:close()
end

return phone
