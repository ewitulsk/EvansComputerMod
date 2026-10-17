# Discord

Discord is the computers' messaging app: servers with text channels and voice
channels, on any number of Discord **backends** a computer can reach. It
replaces the old Chat app and `chatd`; the console `chat` command still works
(it talks in a backend's home server, `#general`).

Two words that sound alike:

| Word | What it is |
|---|---|
| **backend** | A computer program, `discordd`, that hosts servers. One per port; a computer can run several. |
| **server** | What a player sees on the rail in the app ("guild" in the protocol): a community with members and channels. A backend hosts up to 32. |

## Running a backend

```
discordd [PORT] [--name NAME] [--data DIR]
```

- TCP `PORT` (default 7777, the old chat port, so `/etc/chat.conf` still points at
  it) for the app and the `chat` command; UDP `PORT+1` for voice.
- `NAME` names the backend and its **home server** (default `Discord`). Every
  Discord user joins the home server on their first visit; it is owned by nobody,
  so nobody can delete it. Its first text channel is where `chat` users talk.
- `DIR` (default `/var/discord`) keeps the servers, channels, members and the last
  50 messages of every text channel (`DIR/state`, written on every change).
- It announces itself to `disco serve` as `discord` (for the app) and `chat` (for
  the `chat` command), so run `disco serve &` on the same computer to be found
  ([DISCOVERY.md](DISCOVERY.md)).

```
discordd 7777 --name Lab --data /var/lab &
discordd 7790 --name Village --data /var/village &
disco serve &
```

In the Tech Villages the chat village's Data Center runs `discordd 7777 --name
"Tech Village"` at `100.(64+N).0.20` (`discord.ring` / `chat.ring` in DNS), and
every ISP router answers discovery for it ring-wide.

## The app (dev.ecm.discord)

`rust/wasm-programs/discord` (library `discord_app`: `model`, `view`, `app`, `source`).

- **Finds backends by itself.** As it opens (and every minute) it searches the
  network for `discord` services and connects to every backend it finds, plus the one
  in `/etc/chat.conf` (village computers), plus the ones the player added by address
  on the **+** page or that were saved from earlier sessions. A backend that fails is
  retried every 15 s. The first search asks for the `network` permission.
- **Seamless backends.** The rail on the left lists the servers the player joined on
  every backend at once (two letters each; the Discord logo on top goes back to the
  server list). Which backend hosts a server only shows in its subtitle and when
  making one.
- **+ (Add a server):** *Create My Own* (a name; with several backends connected,
  *Host on ...* picks one), *Join a Server* (every server on every connected backend
  the player isn't in, with its member count), the list of backends and their state,
  *Search the network*, and *Add backend* by address (`10.0.8.1` or `10.0.8.1:7790`).
- **A server:** its text channels (`# name`) and voice channels (a speaker icon and the
  name, with the people in it underneath, green while they talk). Its owner sees
  **+ Channel** / **+ Voice** (text channel names become Discord-style: `Build Plans`
  → `build-plans`), and *Delete server*; others see *Leave server*.
- **A text channel:** messages grouped by author, history on opening (the last 50),
  the composer (`Message #name`, Enter or Send; `/nick NAME` changes the nickname on
  every backend). A message from someone else while the window isn't on screen posts
  a notification, "<nick> in #channel (Server)".
- **Members** (wide windows: always on the right; phones: the Members button): online
  first.
- **Settings** (the button by your name): *My Account* with your username. Save (or
  Enter) renames you on every backend, like `/nick`; the page lists what each connected
  backend calls you (one adds a number if the name is taken there).
- **Sizes:** compact (phones) one page at a time (servers and channels, or a channel,
  or members); medium and wide side by side, in Discord's dark colours.

`app.toml`: permissions `network`, `notifications`, `microphone`,
`peripherals:speaker`; background refresh and the `audio` background mode; hotkeys
1 Send, 2 Mute, 3 Hang up. The nickname, the backends and the open server are saved
in the app's data folder. The nickname defaults to `/etc/chat.conf`'s, else
`pc-xxxx` from the computer's MAC.

## Voice channels

Tap a voice channel to join it (one at a time, across all backends). The panel at
the bottom says **Voice Connected**, the channel, where the sound goes
(**Speaker**, or **Headphones** when the phone's audio goes to AirPods,
[BLUETOOTH.md](BLUETOOTH.md)) and **Disconnect**; **Mute** and **Deafen** sit by
your name.

- The app opens the default microphone and speaker (Settings > Sound) at 16 kHz mono:
  on a Terminal a Microphone and a Speaker next to it, on an ePhone its own (or its
  AirPods'). The first join asks for `microphone` and the speaker.
- **Sending:** the microphone is cut into 40 ms frames; a voice gate sends only frames
  louder than -45 dBFS (plus 8 frames after, so words aren't clipped), IMA ADPCM
  (about 8 KB/s while talking), to the backend's UDP port with the session's token.
- **Forwarding:** the backend sends each frame to the others in the channel (a
  selective forwarding unit, like Discord's), and tells everyone who is speaking.
- **Hearing:** each talker gets a small jitter buffer (80 ms before playing); talkers
  are mixed and played about 160 ms ahead.
- **Echo:** on a loudspeaker the app is half duplex, like a speakerphone: while it
  plays someone (and 300 ms after) it doesn't send its microphone, which would hear the
  speaker. With headphones it is full duplex.
- Sound in the room reaches a microphone through the sound field
  ([VOICE.md](VOICE.md)): players talking (with Microphone input on), Speakers playing.

## Protocol (DISCORD/1)

One UTF-8 line per message over TCP; a connection starts in the old chat protocol
(`NICK`, `MSG`, `WHO`, `QUIT`) and a Discord client upgrades it with `HELLO <nick>`.
The full table is in `rust/crates/ecm-discord/src/proto.rs`:

| Client | Backend |
|---|---|
| `HELLO`, `GCREATE`, `GJOIN`, `GLEAVE`, `GDELETE`, `CCREATE`, `CDELETE`, `HISTORY`, `SAY`, `VJOIN`, `VLEAVE`, `NICK`, `PING`, `QUIT` | `READY`, `GUILD`, `GDEL`, `CHANNEL`, `CDEL`, `MEMBER`, `M`, `H`/`HEND`, `VOICE`, `VSTATE`, `RENAME`, `SYNCED`, `PONG`, `ERR` |

Voice datagrams: `"DV"`, version, kind (audio / keep-alive / forwarded), token or
talker id, sequence, samples, IMA ADPCM (`rust/crates/ecm-discord/src/voice.rs`).

## Scenarios and tests

| Scenario | What it shows |
|---|---|
| `discord_app` | A server packages Discord and serves it; a client installs it from the App Store, adds the backend by address (nothing found: no `disco serve`), talks in #general with the server's `chat` command, gets a notification in the background. |
| `service_discovery` | `disco` finds the backend and the repository; Discord connects to the backend it found by itself. |
| `discord_servers` | Two backends on one computer, found by two players; alex makes "Builders" on Village with #build-plans and the Lounge voice channel; bo can't see or change it until joining from Join a Server; bo posts, alex reads; the backend keeps it on disk. |
| `discord_voice` | Two players in General: a Speaker beeping beside alex's Microphone plays on bo's Speaker 30 blocks away; not before; not while alex is muted. |
| `cell_chat`, `cell_tech_village` | A phone installs Discord over LTE; in a Tech Village it finds the ring's backend by itself. |
| `router_chat` | The `chat` command on two LANs against `discordd`. |

Tests: `cargo test -p ecm-discord -p discord -p ecm-chat -p ecm-audio`;
`scripts/Test.ps1 -GameTests ecm_discord`, `-GameTests ecm_apps -GameTestOnly discord_app,service_discovery`;
goldens `discord_compact`, `discord_medium`, `discord_wide` (`cargo run -p discord --example
record_discord_sessions -- ../src/test/resources/ui-sessions`, then `-JUnit UiGoldenTest -McVersion 26.1`).
