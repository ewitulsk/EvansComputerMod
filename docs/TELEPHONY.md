# Telephony: numbers, calls, texts and MMS

As built (PR #87). The plan was `plans/telephony/TELEPHONY_PLAN.md`. This adds phone numbers, VoLTE calls with HD voice, SMS, MMS and group texts, voicemail and a support line, and calls and texts between carriers. It also adds the Phone, Messages and Contacts apps, a `phone` module for Python and Lua, and numbered SIMs in Tech Villages.

Out of scope: landlines, roaming onto another carrier's sites, billing (only CDRs are written) and 911.

Everything runs as programs on computers, as AGENTS.md requires. The carrier's IMS core is four daemons beside `cored`. The phone runs `telephonyd` beside `celld`. Java adds only hardware: the SIM's number and IMS key answers, the modem waking a sleeping phone when it is paged, permission checks at the sockets, and the phone shell's status bar, ringing and HUD.

- [Architecture](#architecture)
- [Numbers](#numbers)
- [The IMS bearer and registration](#the-ims-bearer-and-registration)
- [Calls](#calls)
- [Reaching a phone that is asleep](#reaching-a-phone-that-is-asleep)
- [Call features](#call-features)
- [SMS](#sms)
- [MMS and group texts](#mms-and-group-texts)
- [Voicemail, announcements and 611](#voicemail-announcements-and-611)
- [Between carriers](#between-carriers)
- [The phone: telephonyd](#the-phone-telephonyd)
- [Apps](#apps)
- [Scripts](#scripts)
- [Permissions](#permissions)
- [Tech Villages](#tech-villages)
- [Code map](#code-map)
- [Testing](#testing)
- [Known limits](#known-limits)

## Architecture

```
 phone (ePhone)                                   carrier core computer
 +--------------------------+     LTE      +---------------------------------------------+
 | Phone / Messages /       |   (celld-    | cored: core + HSS/PCRF (TCP 3868)            |
 | Contacts, tel, sms,      |    site)     | imsd:  P/S/I-CSCF, TAS, border, media relay  |
 | phone module             |              |        (SIP 5060, relay 30000-31000)         |
 |   | 127.0.0.1:7720/7726  |  data bearer | smscd: SMS store and forward (SIP 5062)      |
 | telephonyd (IMS UA) -----+--------------+ mmscd: MMSC (SIP 5063, HTTP 8002, MM4 25)    |
 |   | ims0 (IMS bearer)    |  IMS bearer  | mrfd:  voicemail, IVR, conferences           |
 | celld (UE)               |  + voice QCI1|        (SIP 5064, RTP 31000-32000, HTTP 8003)|
 +--------------------------+              +---------------------------------------------+
```

- **cored** is the HSS. It keeps the numbers and each subscriber's IMS profile in `/etc/cored.db`, and answers the IMS daemons over the HSS query protocol (`ecm_cell::hss`). It is also the PCRF: it sets up voice bearers when imsd asks (`AAR`).
- **imsd** is the IMS core. One process plays every CSCF role, the telephony application server, the border to other carriers, and the media relay that carries both legs of every call (`ecm_tel::ims`).
- **smscd** stores and forwards texts (`ecm_tel::smsc`). **mmscd** stores MMS and exchanges it with other carriers (`ecm_tel::mmsc`). **mrfd** plays and records media: voicemail, announcements, the 611 IVR and three-way conferences (`ecm_tel::mrf`).
- **telephonyd** is the phone's IMS client (`ecm_tel::ua`), with its message store and audio. The apps, commands and scripts talk only to telephonyd, never to the network.

The protocol logic lives in sans-IO Rust crates, tested on the host. The programs add only sockets, files and audio:
- `ecm-tel`: SIP, SDP, SMS TPDUs, MMS PDUs, vCard, the CSCF/SMSC/MMSC/MRF/UA machines, and telephonyd's control protocol.
- `ecm-rtp`: RTP, the ADPCM voice codecs, DTMF, the jitter buffer, the echo guard, tones and WAV. The Discord app uses it too.
- `ecm-tel-net`: the WASI side for the daemons.

## Numbers

North American style numbers: area code (NPA), exchange (NXX), line. They're written `(214) 555-0142` and stored as E.164 `+12145550142` (`ecm_tel::number`). The country code is per world (`cc` in cored.db, default 1).

- **Blocks.** A carrier holds blocks of 10,000 numbers, written `block 214-555` in cored.db (`cored block add 214-555`; `cored blocks` lists them with their free lines). [Between carriers](#between-carriers) covers how ianad hands blocks out and publishes them in ENUM.
- **Giving a SIM a number.** The number lives in the subscriber record (`sub ... msisdn=+12145550142`), not on the SIM, as in a real network.
  - `cored sim new` takes the next free line of the carrier's blocks, starting at line 0100 (`FIRST_LINE`), and also writes the number to the SIM. The SIM's copy is only for display: its tooltip and Settings show it before the phone attaches, and the network's copy wins.
  - Subscribers without a number get one the first time cored loads a database that has a block (`fill_numbers`).
  - `cored number <who> [<number>]`, `cored number release <who>`, `cored number move <number> <who>` and `cored numbers` manage them by hand.
- **Short codes and dialling.** `ecm_tel::number::classify` reads what the keypad typed:
  - 7 digits within your own area code, or 10 digits;
  - `*86` voicemail; `611` support; `*67` withhold your number;
  - `*72<number>` / `*73` forward all calls / stop;
  - 5-6 digit short codes, which are texts to services.

## The IMS bearer and registration

A carrier with an `ims pool=... domain=...` line in cored.db gives every phone a second address when it attaches (the `Ims` air item after `AttachAccept`).
- That address can reach only the IMS core: `net`, by default the core's gateway, where imsd runs. It doesn't count against the data quota.
- celld brings it up as the tun `ims0` with a host route to the P-CSCF. Its status shows `msisdn`, `ims_ip`, `pcscf` and `ims_domain`.
- Traffic to the MMSC address is zero-rated on the data bearer too.

telephonyd registers over `ims0`:
1. `REGISTER` gets a 401 with a nonce that carries RAND.
2. The SIM answers it. The SIM tray's `respond_ims` or the Card Reader/Writer applet computes `RES = HMAC-SHA256(K, "ECM-IMS-RES" || RAND)[..16]` and `KS = HMAC(K, "ECM-IMS-KS" || RAND)`. Only the SIM holds K.
3. telephonyd answers with a digest keyed by RES (`ECM-AKAv1-HMAC-SHA256` over `user:realm:nonce:method:uri`).
4. imsd checks it against the vector from the HSS (`MAR`) and records the registration there (`SAR`).

The 200 OK carries the number (`P-Associated-URI`) and the carrier's settings, `P-ECM-Services: mmsc=<url>; vvm=<url>`. The URLs use imsd's `mmsc_http` / `vvm_http` ports. The status bar then shows **VoLTE**, and Settings > Cellular shows **Number** and **Calls**.

Requests from a phone must come from the IMS address registered for its number, so a phone can't send as another. imsd adds `P-Asserted-Identity` itself.

## Calls

- **Setup.** imsd is a back-to-back user agent. The caller's `INVITE` goes to the S-CSCF, through the TAS, and on to the callee's registered contact, a service on mrfd, or another carrier's border.
  - Both legs' RTP go through a relay port pair on imsd (`relay_ports`), so each phone's voice bearer has a fixed far end, and a carrier's phones never see another carrier's addresses.
  - When the call is answered, imsd asks the PCRF for a **voice bearer** (`AAR`). cored adds a QCI 1 bearer matching the call's UDP ports.
  - The cell site gives voice bearers a priority downlink queue and a scheduler sub-channel of their own, up to `max_voice` per cell (default 3). Voice packets also go first in the phone's uplink.
  - The bearer moves with the phone in a handover (`HoRequest.bearers`): a call survives handover with under 100 ms lost (`ecm-cell` e2e test).
- **Voice.** ADPCM in 20 ms RTP frames: wideband 16 kHz (`ADPCM-WB`, payload type 96, HD voice) when both ends offer it, else narrowband 8 kHz (97). Each frame decodes on its own, so a lost packet costs only 20 ms.
  - telephonyd keeps a small jitter buffer of its own, separate from any video playout delay. It plays into the speaker with the mixer role `phone` (#85), through `SpeakerAudio.drain()` like every other sound.
  - The microphone is half-duplex on a loudspeaker (the echo guard) and full duplex on the earpiece or Bluetooth. Keypad tones are RFC 4733 events.
- **In a call.** Hold (`a=sendonly`), mute (local), swap, and merge: a `REFER` with `merge=<call-id>` moves both calls into an mrfd conference. DTMF works both ways.
- **CDRs.** Every call ends with a line in `/var/log/cdr/imsd.cdr`:
  ```
  call start=<s> answer=<s|-> end=<s> dur=<s> from=<n> to=<n> dialled=<what was typed> result=<answered|busy|...> route=<direct|service|border-out|border-in> hidden=<0|1>
  ```
  smscd writes `/var/log/cdr/smscd.cdr` and mmscd writes `/var/log/cdr/mmscd.cdr`. There is no billing.

## Reaching a phone that is asleep

A pocketed phone sleeps (EPHONE.md). To still ring:
- **The UE goes idle.** When `/dev/uictl phone` says the phone is asleep, celld sends `IdleRequest`. The site drops its data context but keeps the subscriber paged by its TMSI. celld then tells the modem `wake tmsi N` on its framed control.
- **Paging wakes the computer.** The core pages the phone when downlink data arrives (an `INVITE`, a text). The modem's wake filter (`FramedRadio`) sees a `Paging` frame (0x09, a count, then big-endian u32 TMSIs) listing its TMSI. It calls `Host.page()` → `IModuleHost.wakeForPage` → `PhoneHost.pageWake`, which wakes the phone's computer for 5 s (`wakeFor`).
- **The network wakes the daemon.** celld resumes, the site re-establishes the context, and the queued packet arrives. telephonyd's sockets are wake sockets (`SO_ECM_WAKE`, #85), so the arriving packet wakes it.
- **Idle again.** A pocketed phone's session can come back up: a page woke it, a call ended, or telephonyd sent something. celld then holds its own wake lock (`wakelock 3000`, renewed every second) until the network has been quiet for 1.5 s and the session is idle again. It releases the lock only after the modem's wake filter is armed, so the next call is paged too (`tel_call_asleep` makes three calls to a phone that stays in the pocket).
- **Staying awake.** While a call rings or is up, telephonyd holds the wake lock (`/dev/uictl wakelock 10000`, renewed every 5 s, `wakelock 0` at the end, #85). The call keeps running in the hotbar or a pocket. App process groups stay duty-cycled under a wake lock, so call audio lives in telephonyd, not in the Phone app.
- **Ringing.** telephonyd rings (`uictl ring 1 <who>`), launches `dev.ecm.dialer --incoming <id>` (a running dialer is only raised and reacts to the `incoming` event), and the holder's HUD says "Incoming call: <who>" every second.

## Call features

The TAS reads each subscriber's profile from the HSS (`UDR`). Operators set features with `cored feature <who> <key> <value>`:

| Key | What |
| --- | --- |
| `cfu`, `cfb`, `cfnr`, `cfnrc` | forward always / when busy / no answer after imsd's `no_answer` (30 s) / not reachable |
| `cw` | call waiting (on by default): a second call rings with a waiting tone |
| `clir` | always withhold your number (`*67` does it for one call; `Privacy: id` too) |
| `dnd` | do not disturb: callers get voicemail or busy |
| `vm` | voicemail (on by default) |
| `block`, `unblock` | the subscriber's block list: blocked callers get "not reachable" |

`*72<number>` / `*73` set and clear `cfu` from the phone.

## SMS

Texts are 3GPP TPDUs (`ecm_tel::sms`: GSM 7-bit or UCS-2, concatenation, status reports) in RP-DATA / RP-ACK. They travel in a SIP `MESSAGE` with type `application/vnd.3gpp.sms`.

- **Store and forward.** A phone sends its `MESSAGE` to imsd, which proxies it to smscd. smscd acknowledges it and delivers to the recipient's registration.
  - If the recipient is off or not registered, smscd keeps the text. It retries when imsd announces the registration (`NOTIFY Event: reg`, body `registered <number> <ip> mms|nomms`) and on a back-off: 30 s, doubling to 10 min.
  - A status report travels back when the text is delivered, so Messages shows Sent → Delivered.
  - Each sender is limited to `rate_per_min` (30).
- **Short codes.** A text to a 5 or 6 digit number goes to a program. The program connects to `127.0.0.1:7730` on the smscd computer and sends `register <code>`, then answers each `sms <code> <from> <text>` line with `reply <code> <to> <text>` (percent-encoded).
  - The carrier's own balance service answers on 22225: text `BAL`.
- **Service texts.** mrfd's 611 menu sends texts as the carrier (`service-text`). MMS notifications are WAP push: a binary SMS to port 2948, sent to smscd as `sip:wap-push@`.

## MMS and group texts

OMA MMS PDUs (`ecm_tel::mms`).
- A phone POSTs an `M-Send.req` to the MMSC URL over its data bearer.
- mmscd finds the sender by the source address. It asks the HSS (`IPD`, header enrichment), so a phone can't send as another.
- It stores the message and sends each recipient an `M-Notification.ind` by WAP push.
- The phone's telephonyd fetches the message (GET `/mms/<id>`, auto-download), acknowledges it (`M-NotifyResp.ind`), and saves the parts under `/home/Messages/<id>/`.
- A **group** is an MMS to several numbers. Its thread key is the sorted numbers joined by commas, without your own, and every member's reply goes to all.
- **Delivery reports** come back as `M-Delivery.ind`.
- A recipient on a phone without MMS gets a text with a Browser link (`/v/<id>?r=<token>`). The link key is kept in `/var/mmscd/link.key`, so old links survive a restart.

## Voicemail, announcements and 611

mrfd answers SIP on 5064 and offers services (`sip:<service>@mrf`):
- `voicemail`: the greeting, then a recording of up to `max_message` seconds. It sends a waiting indication (`NOTIFY` message-summary).
- `voicemail-retrieve`: dial `*86`. Keys: `7` delete, `9` save, `1` replay, `#` next.
- `announcement`: early media (183) for "the number you have called...", busy and similar.
- `support`: dial `611`. Press `1` for your data used and quota as a text, `2` for your number as a text.
- `conference`: three-way calls.

Visual voicemail is HTTP on 8003 (`/vm`). The Phone app's Voicemail tab lists, plays, marks heard and deletes messages through telephonyd (`voicemail`, `voicemail-fetch`, ...).

## Between carriers

- **Peering.** Each pair of carriers that exchange calls and texts adds the other on both sides, with the same secret: `imsd peer add <domain> <ip[:port]|srv> <secret>` and mmscd's `peers`. Requests across carry `P-ECM-Peer: <domain>;proof=HMAC(secret, "ECM-NNI" || domain || call-id)`, and a request from a peer that isn't configured gets 403. `public_peering = true` also accepts carriers that publish ENUM and SRV, without a secret.
- **Finding the other carrier: ENUM.** For a number that isn't its own, imsd asks DNS for NAPTR at the number's name under `e164.arpa` (`2.4.1.0.5.5.5.4.1.2.1.e164.arpa`), then at its block's name. It then looks up the carrier's border with SRV `_sip._udp.<domain>`. `dig -t NAPTR` shows the records.
- **Texts** go to the destination carrier's border, which hands them to its smscd: the destination's SMSC delivers. A 202 means a peer SMSC took the text over.
- **MMS** goes between MMSCs as MM4 over SMTP (port 25, found by MX). The message is a base64 `M-Retrieve.conf` with an `X-ECM-Peer` proof (`mm4_proof`).
- **Voice** always goes through both carriers' media relays.

### Number blocks and porting (ianad)

ianad, the DNS root's TLD desk, is also the numbering administrator. It keeps the zone `e164.arpa` on IANA's `named`, delegated from the root (`e164.arpa. NS iana.ring.`), so any resolver reaches it. [DNS.md](DNS.md) has the records and the API.

1. A carrier publishes its SIP border: `_sip._udp.<domain>` SRV, or at least the domain's A record (`domain set cell-red.ecm @ A <ip>`).
2. It applies for a block: `domain enum apply +1214555 cell-red.ecm`. ianad checks that the block is free and the domain answers, publishes a NAPTR at the block's name, and prints the carrier's key once. The carrier then runs `cored block add 214-555` and `cored enum-key <key>`. Its later blocks use the same key.
3. **Porting a number from carrier A to carrier B:**
   - A makes a port-out code for the subscriber: `cored port-code <who>`. It prints a code like `3fa2c1-b843a26d`, a nonce and an HMAC of the number under A's key. Each code works once.
   - B gives the number to its new SIM (`cored number <sim> +12145550142 ported`), then asks ianad to move it: `domain enum port +12145550142 cell-blue.ecm <code> <B's key>`.
   - ianad checks the code with A's key and writes a per-number NAPTR that overrides the block.
   - A releases its old subscriber's number (`cored number release <who>`). Calls from A's own phones then find B through ENUM like anyone else's.
   - Porting back to the block's holder removes the override.

Check with `domain enum list` and `dig -t NAPTR 2.4.1.0.5.5.5.4.1.2.1.e164.arpa`.

## The phone: telephonyd

PhoneUi starts telephonyd beside celld, once celld runs: a modem and a SIM are in. telephonyd:
- polls celld's status every second for the IMS bearer;
- registers and keeps the registration fresh;
- runs calls and audio (`ecm_audio` device and microphone, mixer role `phone`), and records or plays WAVs into calls for scripts;
- sends and receives texts and MMS, and keeps the message store in `/var/telephony` (`messages.log`, `calls.log`; apps can't read it);
- tells the shell what to show through `/dev/uictl`: `volte <0|1> <number>`, `ring <0|1> <who>`, and `notify <app> <tag> <title> <body>`, which posts a system notification;
- holds the wake lock during calls.

Tapping one of its notifications opens the app with `NotificationAction{tag}`: `call:missed` opens Recents, `vm` Voicemail, `sms:<thread>` the thread, `sms:failed` the list. On a phone the app is raised.

**Control protocol** (`ecm_tel::ctl`): TCP on loopback, framed like the cellular daemons' admin sockets. A connection sends one request line and gets `ok` / `err <why>`, body lines, then `.`. Text fields are percent-encoded. Port **7720** takes everything; port **7726** takes only `status` and the text requests (`sms`, `mms`, `threads`, `messages`, `read`, `delete`, `delete-thread`, `watch`).

| Request | Reply body |
| --- | --- |
| `status` | `registration msisdn domain carrier bars volte hd voicemail unread missed calls mmsc speaker now` (`now`: telephonyd's clock, the one every time in replies and events uses) |
| `calls` | `call <id> <state> <number> <hidden> <outgoing> <seconds> <muted> <hd> <codec> <loss%> <merged>` |
| `dial <number> [hide]` | `call <id>` |
| `answer`, `hangup`, `decline <id>`; `hold`, `mute <id> on\|off`; `swap`; `merge`; `dtmf <id> <digits>`; `speaker on\|off` | |
| `play <id> <wav>`, `record <id> <seconds> <wav>` | WAV into or out of a call (files under `/home/` or `/tmp/` only) |
| `sms <to> <text>` | `sms <id>` |
| `mms <to,to,...> <text\|-> <file,file\|->` | `mms <id>` |
| `threads`, `messages <key> [n]`, `read <key>`, `delete <id>`, `delete-thread <key>` | threads newest first; `msg <id> <in\|out> <from> <time> <status> <sms\|mms> <text> <files>` |
| `log [n]` | `call <id> <in\|out\|missed> <number> <start> <seconds> <reason>` |
| `voicemail`, `voicemail-fetch <id> <path>`, `voicemail-heard <id>`, `voicemail-delete <id>` | `vm <id> <from> <time> <ms> <new\|heard>` |
| `forward all <number>` / `forward off` | sends `*72` / `*73` |
| `watch` | events: `incoming`, `call`, `sms`, `mms`, `sent`, `delivered`, `failed`, `dtmf`, `voicemail`, `registration` (on 7726 only the text and registration events) |

## Apps

All three are built-in phone apps (PhoneUi `BUILTINS`), granted their permissions at install. They're Rust on `ecm-ui`, in the crate `ecm-tel-ui`, with compact, medium and wide layouts and goldens. They use no network themselves.

| App | Program, permissions | Screens |
| --- | --- | --- |
| Phone `dev.ecm.dialer` | `dialer`; phone, contacts, notifications | Favourites, Recents (missed in red, count on the tab), Contacts, Keypad (live formatting, contact match, keyboard input), Voicemail. Full-screen incoming call: Answer / Decline / Message, with Answer and Decline on hotkeys 1 and 2 while ringing; with call waiting, End & answer and Hold & answer. In-call: name, timer, HD, codec and loss; mute, keypad, speaker, add call, hold, swap, merge, end. Args `--incoming <id>`, `--number <n>`, `--call <n>`. |
| Messages `dev.ecm.messages` | `messages`; sms, contacts, files:documents, microphone, notifications | Conversation list (unread counts, previews). Threads: bubbles with Sending / Sent / Delivered / Failed (Retry), sender names in groups, pictures inline (full screen, Save to Files), voice notes, contact cards (Add to Contacts). Compose: contact suggestions, recipient chips, a character and part counter, attach a picture, a recorded voice note or a contact. One recipient with text only goes as SMS, anything else as MMS. Args `--to`, `--thread`, `--text`, `--vcard`. |
| Contacts `dev.ecm.contacts` | `contacts`; contacts, sms (for My card's number), files:documents | List with search and A-Z, My card, contact page (labelled numbers with Call and Text, email, notes, photo), edit and add, delete, favourite, share as `.vcf`, import `.vcf` (the default handler for `.vcf`). Args `--add <n>`, `--show <n>`, a `.vcf` path. |

The address book is `/home/.contacts/contacts.vcf` (vCard 3.0, `ecm_tel::vcard`; favourites are `X-ECM-FAVOURITE:1`). Phone, Messages and the `phone` module share it.

## Scripts

Python and Lua have a `phone` module with the same names in both (the parity test checks it):
- `number`, `status`;
- `sms_send`, `sms_status`, `sms_inbox`, `threads`, `messages`, `mark_read`, `mms_send`;
- `dial`, `calls`, `speaker`, `call_log`;
- `on_sms`, `on_mms`, `on_call`, `poll`, `run`;
- `contacts`, `add_contact`, `PhoneError`, `Call`.

A `Call` has `info`, `state`, `wait`, `answer`, `reject`, `hangup`, `hold`, `mute`, `send_tones`, `tones`, `play` and `record`. Events come on a `watch` connection that is a wake socket (`net.set_wake`). `poll(seconds)` uses the new `net.tcp_ready(conn, ms)`. See `man telephony` and `man scripting`.

## Permissions

`contacts`, `phone` and `sms` (APPS.md):
- **Sockets.** `WasiFunctions.sock_connect` checks them at telephonyd's ports: 7720 needs `phone`, 7726 needs `sms`.
- **Files.** `AppManager.checkPath` gives `/home/.contacts` only to `contacts`, and `/var/telephony` to no app.
- **Confirmation.** Each call, text or MMS that a non-system app sends is confirmed by the player (`AppManager.confirm`, the prompt `confirm:<call|sms|mms>:<to>`, timing out as a no).

telephonyd itself reads and writes the files a client names only under `/home/` or `/tmp/` (not the address book), so it can't be used to reach other files.

## Tech Villages

`CellPlan` adds telephony to the world's carrier:
- **Numbers.** Every village gets its own area code, picked by the seed from a list of real NANP codes, with exchange 555: block `NPA-555`. Every Phone Store SIM is numbered: the phones on display get lines 0100-0103 and the chest's SIMs 0104 onwards. The SIM item shows its number.
- **cored.db** gets the blocks, `ims pool=10.250.0.0/22 pcscf=100.80.0.1 domain=<carrier>.ecm net=100.80.0.1/32`, and every SIM's `msisdn`.
- **The core's `services.cfg`** also starts `imsd`, `smscd`, `mmscd` and `mrfd` (`start --wait &`). The carrier's IMS core runs on the core computer, so any two store phones can call and text.
- **The Phone Store sign** shows the area code and "help: call 611".

## Code map

| Piece | Where |
| --- | --- |
| Numbers, IMS profile, blocks, HSS protocol, IMS AKA, IMS/voice bearers, paging and idle | `rust/crates/ecm-cell` (`db.rs`, `hss.rs`, `simauth.rs`, `air.rs`, `backhaul.rs`, `site.rs`, `sched.rs`, `core.rs`, `ue.rs`) |
| SIP, SDP, SMS, MMS, vCard, the CSCF/SMSC/MMSC/MRF/UA machines, control protocol | `rust/crates/ecm-tel` |
| Voice: RTP, codecs, DTMF, jitter buffer, echo guard, tones, WAV | `rust/crates/ecm-rtp` |
| Daemons' WASI side (sockets, HTTP, SMTP, DNS, HSS client) | `rust/crates/ecm-tel-net` |
| ENUM records (NAPTR, SRV, `enum_dns`) | `rust/crates/ecm-dns` |
| Programs | `rust/wasm-programs/{imsd,smscd,mmscd,mrfd,telephonyd,tel,sms,dialer,messages,contacts}`; `cored`, `celld` |
| Apps | `rust/crates/ecm-tel-ui` |
| Python / Lua module | `rust/wasm-programs/python/src/phone.py`, `rust/wasm-programs/lua/src/lua/phone.lua` |
| Java | `card/` (SIM number, IMS answers), `radio/framed/FramedRadio` (paging wake), `phone/PhoneHost` (`pageWake`), `computer/ui/PhoneUi` (telephonyd, VoLTE, ringing, built-ins), `computer/ui/UiManager` (`uictl volte/ring/notify`), `computer/ui/Compositor` (notification taps), `computer/apps/AppManager`, `computer/wasi/WasiFunctions` (permissions), `computer/CellPlan` (Tech Villages) |

## Testing

- **Host:**
  - `cargo test -p ecm-tel`: the carrier and phone machines end to end over a simulated network, covering registration, calls, features, SMS, MMS, voicemail and peering.
  - `-p ecm-cell`: IMS and voice bearers, handover with a call, idle and paging.
  - `-p ecm-rtp`, `-p ecm-tel-ui` (the apps), `-p telephonyd`, `-p imsd`.
  - `-p python` and `-p lua` for the phone module and parity, and `-p ecm-man`.
- **UI goldens** (`UiGoldenTest`): `dialer_*`, `messages_*`, `contacts_*`, `settings_cellular*`.
- **Java:** `CellPlanTest`.
- **Scenarios** (`/ecm scenario`, GameTests `ecm_telephony`): `tel_register`, `tel_call` (with a video playing during the call), `tel_call_asleep`, `tel_sms`, `tel_mms_group`, `tel_scripts`, `tel_ivr`, `tel_interconnect` (two carriers, ENUM, peering, porting); `tel_tech_village` runs in the tech client suite. See TESTING.md.

## Known limits

- telephonyd's MMS sends and fetches and its visual-voicemail requests are blocking HTTP inside its main loop (up to 30 s). A call in progress stalls meanwhile.
- **Display pipeline (#86):** call audio is live: a speaker drain that carries the `phone` role goes out at once with `SpeakerAudioPacket.FLAG_LIVE`, not on a video's playout delay (`tel_call` checks it while a video plays). telephonyd still writes its 20 ms frames straight into the mixer; its RTP timestamps are not mapped to the shared clock.
- No emergency calls, no roaming, no landlines, and no billing beyond CDRs.
