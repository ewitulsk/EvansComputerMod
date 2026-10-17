# Music

Music is the computers' and ePhones' music player. Songs come from **music
servers** (`musicd`): computers whose owner put MP3 files in a library folder.
The Music app finds them on the network by itself, plays them as a stream, and
keeps playing in the background or in the pocket.

| Piece | What it is |
|---|---|
| `musicd` | The music server program: library, search, playlists, likes, streaming, cover art. |
| Music (`dev.ecm.music`) | The app: Home, Search, Library and Now Playing, on any number of servers. |

## For players: the app

Open **Music**. It looks for music servers on the network when it opens and
again every minute (run `disco serve &` on the server's computer so it can be
found). A server it can't find by itself can be added in
**Settings > Add a server** (`10.0.0.5` or `radio.village:7800`). The servers
are listed on the left (on a phone: on Home and in Settings), each with your
name when you are signed in, or **Sign in**.

- **Home**: recently played, your playlists, and what is new on the server.
- **Search**: results appear as you type (`daft pu` finds Daft Punk). Typing
  mistakes are forgiven; `artist:`, `album:`, `genre:` and `year:` narrow the
  songs (`year:1990-1999`, `artist:"daft punk"`). The answer is grouped: the
  top result, then songs, artists, albums and playlists.
- **Library**: your playlists, Liked Songs, all albums and artists.
- **Now Playing**: the cover, the seek bar, play/pause, previous/next,
  shuffle, repeat (off, all, one) and the queue.

Tapping a song plays it and puts the list it was in into the queue. Each song
also has **Like**, **Next** (play it next), **Queue** (add it at the end) and
**+** (add it to a playlist, or to a new one). Your own playlists can be
renamed, reordered, made public or private, and deleted. Other players only see
a playlist while it is public.

Browsing and searching usually work without an account (the server's owner can
turn that off). Playing, playlists and likes need you to sign in.

### Playing in the background

Music keeps playing when you go Home, open another app, put the phone in the
hotbar or in your main inventory (it asks once whether it may play in the
background). In a chest, on the ground or with an empty battery it stops. The
Now Playing card (and the ePhone's hotkeys: 1 play/pause, 2 previous, 3 next)
control it while another app is open. A Discord call that carries sound lowers
the music while people talk.

If the phone is switched off, Music remembers the queue and where you were and
opens there again, paused.

### Streaming quality

**Settings > Streaming quality**:

| Setting | What it streams |
|---|---|
| Auto (default) | Data saver on cellular, Original on Wi-Fi or a cable. |
| Original | The MP3 as the owner uploaded it (128 kbps is about 16 KB/s). |
| Data saver | 22 kHz mono, about 11 KB/s, made once per song by the server. |

A change applies from the next song. A song whose Data saver copy the server
hasn't made yet plays in Original.

Music starts a song once it has 3 seconds of it, then keeps 30 seconds ahead, so
a slow or busy server doesn't interrupt it. If the music runs dry it shows
**Buffering...** and goes on from the same moment once enough is back: nothing
is skipped. A dropped connection is picked up where it stopped. While a song
is far enough ahead, the start of the next song in the queue is fetched too,
so it starts at once.

### Accounts

Each server either keeps its own accounts or uses a shared Auth server
(`authd`), so one account works on many servers. The first time you sign in to
a server's accounts, Music shows their name, address and **key**; compare it
with the one the owner gives out. Music remembers that key. If a server with
that name or address later shows another key, Music refuses to sign in there
("The identity of ... has changed") and never offers your saved sign-in to it.
If the owner tells you the key really changed, remove the remembered key in
**Settings > Accounts (Forget)** and sign in again. Your password never leaves
the phone. See [AUTH.md](AUTH.md) for how this works.

## For owners: running musicd

```
musicd [PORT] [--name NAME] [--library DIR] [--data DIR]
       [--auth local | --auth HOST:PORT --auth-key FINGERPRINT]
       [--transcode on|off] [--public-browse on|off] [--max-kbps N]
musicd scan | status | key
```

- HTTP on `PORT` (default 7800). With `--auth local` (the default) musicd keeps
  its own accounts and players sign in on `PORT+1`. With `--auth HOST:PORT
  --auth-key FINGERPRINT` it trusts only that `authd` (`authd key` prints the
  fingerprint).
- Put MP3s anywhere under the library folder (default `/srv/music`), with
  `upload` or `cp`. musicd scans at start and checks the folder every 30
  seconds; `musicd scan` scans at once. Song ids come from the file's path:
  moving a file makes it a new song (playlists keep the old one, shown as no
  longer on this server).
- Tags (title, artist, album, album artist, track, year, genre) and the cover
  picture come from the files' ID3 tags; a file without a title is named after
  the file.
- `--transcode off` serves Original only. Otherwise musicd makes the Data saver
  copies in the background, a little at a time.
- `--max-kbps` (default 192) lists bigger files as warnings in `musicd status`.
- `--public-browse off` asks for a sign-in for browsing and search too.
- The data folder (default `/var/music`) holds the index, the playlists, likes
  and play counts (`store`), the server's key (`backend.key`), the local
  accounts (`users`), the cover pictures (`art/`) and the Data saver copies
  (`cache/`).
- Run `disco serve &` too, so Music apps find the server.

```
musicd --name "Tech Village Radio" &
disco serve &
musicd status
```

The HTTP API (for programs and scripts) is described in the plan
(`plans/background-apps/MUSIC_AND_AUTH.md`, "API") and `man musicd`.
