# Microphones and voice

Computers hear sound the way real ones do: a microphone is a capture device
programs read PCM from, and a radio carries a voice as a modulated signal on
the radio medium. Nothing is shortcut: a voice that reaches a computer or a
radio has travelled through the air (with distance) or over the air (with the
link budget, noise and interference of every other radio signal).

## Hardware

| Hardware | What it is |
|---|---|
| **Microphone** (block) | A desk microphone. Next to a Terminal it is a `microphone` peripheral and `/dev/mic`. Its tally lamp lights while a program records. Recipe: wool over a copper ingot over three iron ingots. |
| **ePhone microphone** | Every ePhone has one (peripheral `microphone`, `/dev/mic`), at the holder's hand. |
| **Handheld Radio** | Receives AM, shortwave and VHF FM as before, and transmits 5 W narrow FM on VHF two-way channels (not the 88-108 MHz broadcast band) while its holder holds **Talk**. Half duplex: it hears nothing while it talks. |
| **Speaker** | Unchanged for listeners, and now also a sound source microphones hear. |
| **AirPods** | Paired with an ePhone and worn, they are its speaker and microphone over Bluetooth: the phone's sound plays only in the wearer's ears (it is not in the air), and `/dev/mic` hears the wearer's mouth ([BLUETOOTH.md](BLUETOOTH.md)). |

Discord's voice channels use these devices ([DISCORD.md](DISCORD.md)).

## The sound in the air

Every source of sound (a talking player, a playing Speaker, an ePhone's
speaker) is put into the server's sound field at its position. A microphone
hears the sum of the sources around it, each fainter with distance: full level
within one block, then half as loud at twice the distance (the inverse-distance
law), and nothing past `microphone.range` (24 blocks by default). So a
Microphone two blocks from a Speaker records it at about -6 dB, a player
talking in the next room is quieter than one at the desk, and a program that
plays what it records next to its own Microphone feeds back, as it would in a
real room.

Sound arrives in bursts (a client sends its microphone every tick), so each
burst is heard 100 ms after it arrived, back to back with the one before, like
a VoIP jitter buffer.

## /dev/mic

A capture device, the counterpart of `/dev/audio`:

```
/dev/mic, /dev/mic.<side>          read PCM
/dev/micctl, /dev/micctl.<side>    write: rate N | bits 8|16 | channels 1|2 | gain 0-100 | flush
                                   read:  rate bits channels gain buffered_ms overruns level_db listening
```

- **Format**: 8000-48000 Hz, 8-bit unsigned or 16-bit signed little-endian,
  mono or stereo (both channels carry the one capsule). Format and gain belong
  to the card and are shared by everything reading it.
- **Gain** (input volume) 0-100: 50 is the capsule's own level, every 25 is
  10 dB, 0 mutes. The Microphone block saves it.
- **Streams**: each open `/dev/mic` is its own capture stream, so two programs
  recording at once both get everything (a level meter doesn't steal audio
  from a recorder).
- **Clock**: a stream starts capturing with its first read and runs at its
  rate from then on. A read waits until the samples it asked for have been
  captured (at most half a second per read); non-blocking reads return what is
  there or `EAGAIN`. A program that reads less than once a second loses audio
  and counts an `overrun`. A quiet room reads as the capsule's faint hiss
  (about -75 dBFS), not digital zero.
- **Permission**: apps need `microphone` (also for the `microphone` peripheral).

Programs and libraries:

- `arecord FILE.wav [-d SECONDS] [-r RATE] [-b 8|16] [-c 1|2] [--mic SIDE] [--gain N]`
  records a WAV (valid after every block, so Ctrl-C keeps what was recorded);
  `arecord --level` prints a live level meter.
- `radio_station /dev/mic[.side] <freq> [--mode fm|am|wbfm] [--power DBM] [--duration S]`
  transmits a microphone live through an SDR: a computer talking on the radio.
- Rust: `ecm_audio::Mic` (`open`, `set_format`, `set_gain`, `read_samples`, `status`).
- Python and Lua (`audio` module, same functions): `audio.open_mic(side)` with
  `set_format`, `set_gain`, `read`, `record(seconds)`, `level`, `status`,
  `flush`, `close`; plus `audio.peak_db`, `audio.save_wav`, `audio.wav_header`
  and `audio.default_device("input"|"output")`.

## Settings > Sound

The Settings app's Sound page lists the attached speakers and microphones,
chooses the default ones (written to `/etc/sound.conf` as `output <side>` /
`input <side>`; `/dev/audio`-opening programs, `ecm_audio`, Python and Lua use
it when no side is given), sets their volume and gain, and shows a live input
level meter (it records from the microphone while the page is open).

## Your microphone (client)

Each player decides whether the mod may hear them: **Microphone input** in
Mods > Evans Computer Mod > Config (`config/evanscomputermod-voice-client.toml`),
off by default. While it is off the microphone is never opened.

When it is on, the server tells the client what wants the player's voice (a
Microphone within range that a program is reading, or a switched-on Handheld
Radio in hand) and the microphone is opened only then. A HUD indicator shows
`MIC` (red dot while the voice is sent) or `TX` while the radio transmits.

- **Push to talk** (default): you are heard only while you hold **Talk** (B).
- **Voice activation**: you are heard when you speak louder than
  `activationLevelDb`, while a microphone near you records.
- A Handheld Radio always transmits only while Talk is held (its PTT).

The voice goes to the server at 24 kHz, IMA ADPCM (about 12 KB/s while you
talk). Server operators can refuse players' voices with `microphone.playerVoices`.

### Proximity voice chat mods

ECM never opens the microphone a second time next to a voice chat mod:

- **Simple Voice Chat**: ECM is a Simple Voice Chat plugin and takes the audio
  Simple Voice Chat already captures (`ClientSoundEvent`). Simple Voice Chat
  decides when the microphone is live (its own push-to-talk or voice
  activation), so to talk on an ECM radio hold ECM's Talk *and* talk in Simple
  Voice Chat. Players near you still hear you through Simple Voice Chat as
  usual; ECM doesn't change or block its audio.
- **Plasmo Voice**: ECM is a Plasmo Voice client add-on and takes each frame
  Plasmo Voice captures (`AudioCaptureEvent`); ECM's own push-to-talk / voice
  activation decides what is sent to ECM. Plasmo Voice's own audio is left as
  it is.
- Without either, ECM uses OpenAL capture (the OpenAL Minecraft already ships,
  so no extra native code), 48 kHz mono, opened only while needed.

## Radio voice

A Handheld Radio's transmitter takes its microphone (the sound field at the
holder's mouth, so the holder first and anything loud nearby), band-limits it
to 3 kHz, pre-emphasises it (75 µs, matching the receivers' de-emphasis),
FM-modulates it at ±5 kHz deviation and puts it on the medium as an I/Q
emission at 37 dBm on its channel. Any receiver hears it as it hears anything
else: another Handheld Radio, an SDR running `rx_fm`, a scanner, with range,
walls, fading and other transmitters on the same channel. A server can veto it
like any transmission (`RadioTransmitEvent`, kind `handheld`).

## Scenarios and tests

| Scenario | What it shows |
|---|---|
| `mic_record` | A Microphone records a Speaker's beep (`arecord`, Python and Lua `audio.open_mic`); quiet when nothing plays; the tally lamp. |
| `mic_voice` | A Microphone records a player talking two blocks away, not from 40 blocks; the server asks for the player's voice only while a program records. |
| `radio_voice` | Push-to-talk on a Handheld Radio heard on another 24 blocks away (not before Talk, not 30 kHz off); a computer talking through `radio_station /dev/mic`. |

Tests: `scripts/Test.ps1 -JUnit MicAudioTest -McVersion 26.1` (sound card,
sound field, modulator against the handheld's demodulator);
`cargo test -p ecm-audio`, `-p lua --lib`, `-p python --lib audio`,
`-p ecm-apps --test apps settings`, `-p ecm-radio --lib cli`;
`scripts/Test.ps1 -GameTests ecm_voice` (the three scenarios).
