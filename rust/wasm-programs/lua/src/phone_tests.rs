//! lua/phone.lua on a real Lua against a scripted telephonyd (a stand-in `net`
//! whose connections answer with telephonyd's control protocol, ecm_tel::ctl):
//! the requests the module sends, the replies it parses, events through poll(),
//! and the address book file.

use super::*;

fn lua() -> Lua {
    let lua = Lua::new();
    install(&lua).expect("install");
    lua
}

fn eval(lua: &Lua, code: &str) -> String {
    lua.load(code).set_name("=test").eval::<String>().unwrap_or_else(|e| panic!("lua raised:\n{e}\nin:\n{code}"))
}

/// A `net` with telephonyd behind 127.0.0.1:7720 / 7726: `sent` records requests
/// (port and line), `push(line)` queues an event for the watch connection.
const FAKE_TELEPHONYD: &str = r#"
sent = {}
local conns, next_fd, events = {}, 1, {}
local replies = {
  status = "ok\nregistration registered\nmsisdn +12145550142\ncarrier cell-red\nbars 4\n.\n",
  threads = "ok\nthread +12145550143 1 9000 See%20you\nthread +12145550143,+12145550144 0 8000 Hi\n.\n",
  ["messages +12145550143 200"] = "ok\nmsg 7 out +12145550142 5000 delivered sms On%20my%20way -\nmsg 8 in +12145550143 9000 received sms See%20you -\n.\n",
  ["messages +12145550143 50"] = "ok\nmsg 7 out +12145550142 5000 delivered sms On%20my%20way -\nmsg 8 in +12145550143 9000 received sms See%20you -\n.\n",
  ["messages +12145550143,+12145550144 200"] = "ok\nmsg 9 in +12145550144 8000 received mms Hi /home/Messages/9/a.png\n.\n",
  ["messages +12145550143,+12145550144 50"] = "ok\nmsg 9 in +12145550144 8000 received mms Hi /home/Messages/9/a.png\n.\n",
  ["sms +12145550143 On%20my%20way"] = "ok\nsms 7\n.\n",
  ["dial 611"] = "ok\ncall 3\n.\n",
  calls = "ok\ncall 3 active 611 0 1 42 0 1 ADPCM-WB 0.0 0\n.\n",
  ["dial %2D"] = "err not a number\n",
}
fake_net = {
  tcp_connect = function(host, port)
    assert(host == "127.0.0.1")
    local fd = next_fd; next_fd = next_fd + 1
    conns[fd] = { port = port, out = "" }
    return fd
  end,
  tcp_send = function(fd, data)
    local c = conns[fd]
    for l in data:gmatch("(.-)\n") do
      sent[#sent + 1] = c.port .. " " .. l
      if l == "watch" then c.watch = true; c.out = c.out .. "ok\n"
      else c.out = c.out .. (replies[l] or "ok\n.\n") end
    end
    return #data
  end,
  tcp_ready = function(fd) local c = conns[fd]
    if c.watch and #events > 0 then c.out = c.out .. table.remove(events, 1) .. "\n" end
    return c.out ~= "" end,
  tcp_recv = function(fd, n) local c = conns[fd]
    if c.watch and c.out == "" and #events > 0 then c.out = table.remove(events, 1) .. "\n" end
    local s = c.out:sub(1, n); c.out = c.out:sub(n + 1); return s end,
  tcp_close = function(fd) conns[fd] = nil end,
  set_wake = function(fd, on) conns[fd].wake = on end,
}
function push(line) events[#events + 1] = line end
phone = require("phone")
phone._net = fake_net
"#;

#[test]
fn texts_calls_and_events_through_telephonyd() {
    let l = lua();
    l.load(FAKE_TELEPHONYD).exec().unwrap();
    assert_eq!(eval(&l, "return phone.number() .. ' ' .. phone.status().bars"), "+12145550142 4");
    assert_eq!(eval(&l, "return tostring(phone.sms_send('+12145550143', 'On my way'))"), "7");
    assert_eq!(eval(&l, "return phone.sms_status(7)"), "delivered");
    let inbox = eval(&l, "local t = {} for _, m in ipairs(phone.sms_inbox()) do t[#t + 1] = m.from .. ':' .. m.text .. ':' .. #m.files end return table.concat(t, '|')");
    assert_eq!(inbox, "+12145550143:See you:0|+12145550144:Hi:1");
    let c = eval(&l, "local c = phone.dial('611') c:send_tones('1#') local i = c:info() return c.id .. ' ' .. c:state() .. ' ' .. i.seconds .. ' ' .. tostring(i.hd) .. ' ' .. i.codec");
    assert_eq!(c, "3 active 42 true ADPCM-WB");
    let e = eval(&l, "local ok, e = pcall(phone.dial, '-') return tostring(e)");
    assert_eq!(e, "phone: not a number");
    // Events: a text, an MMS in a group, an incoming call answered, the far end's keypad.
    let got = eval(
        &l,
        r#"local got = {}
phone.on_sms(function(from, text) got[#got + 1] = "sms " .. from .. " " .. text end)
phone.on_mms(function(from, group, text, files) got[#got + 1] = "mms " .. from .. " " .. table.concat(group, ",") .. " " .. text .. " " .. files[1] end)
phone.on_call(function(c) got[#got + 1] = "call " .. c.id .. " " .. c.number; c:answer() end)
push("sms id=10 from=+12145550143 thread=+12145550143 text=Hello%20there")
push("mms id=11 from=+12145550144 thread=+12145550143,+12145550144 text=Look files=/home/Messages/11/a.png")
push("incoming call=5 from=+12145550145 hidden=0 waiting=0")
push("dtmf call=5 digit=4")
local n = phone.poll(0)
got[#got + 1] = n .. " " .. phone.calls()[1].id
return table.concat(got, "|")"#,
    );
    assert_eq!(got, "sms +12145550143 Hello there|mms +12145550144 +12145550143 Look /home/Messages/11/a.png|call 5 +12145550145|4 3");
    let sent = eval(&l, "return table.concat(sent, '|')");
    assert!(sent.contains("7726 sms +12145550143 On%20my%20way"), "texts go on the sms port: {sent}");
    assert!(sent.contains("7720 dial 611") && sent.contains("7720 dtmf 3 1#") && sent.contains("7720 answer 5"), "{sent}");
    // The watch moved to the phone port once on_call was registered.
    assert!(sent.contains("7726 watch") && sent.contains("7720 watch"), "{sent}");
}

#[test]
fn the_address_book_is_the_contacts_vcard_file() {
    let d = std::env::temp_dir().join(format!("ecm-lua-phone-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let l = lua();
    l.load(FAKE_TELEPHONYD).exec().unwrap();
    let path = d.join(".contacts/contacts.vcf").to_string_lossy().replace('\\', "/");
    l.globals().set("BOOK", path.clone()).unwrap();
    let out = eval(
        &l,
        r#"phone.CONTACTS = BOOK
phone.add_contact("Bo Jones", "+12145550143")
phone.add_contact("Alex, the Builder", "+12145550144", "work")
local t = {}
for _, c in ipairs(phone.contacts()) do t[#t + 1] = c.name .. "=" .. c.numbers[1].label .. ":" .. c.numbers[1].number end
return table.concat(t, "|")"#,
    );
    assert_eq!(out, "Alex, the Builder=work:+12145550144|Bo Jones=cell:+12145550143");
    // Contacts (ecm_tel::vcard) reads the same file.
    let cards = ecm_tel::vcard::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[1].name, "Alex, the Builder");
    let _ = std::fs::remove_dir_all(&d);
}
