# Apps — EvansComputerMod

The app platform (roadmap [P4](ROADMAP.md#p4--app-platform)): packages, the `pkg` package manager and repositories, the app manager, permissions, saved state and the ePhone background rules, plus its user-facing programs: the [App Store](#app-store), the [file picker and the open-with chooser](#file-picker-and-chooser) and the [Settings → Apps section](#settings--apps). The permission dialog belongs to the desktop shell and plugs into the interfaces in [For the compositor and the phone](#for-the-compositor-and-the-phone).

This page is the one schema document for `app.toml`, `.ecmpkg` and `index.json`. Two implementations follow it and run the same vectors in [`abi/app-manifest-vectors`](../../abi/app-manifest-vectors): `rust/wasm-programs/pkg/src` (`ecm_pkg`) and `computer/apps/AppManifest.java` + `MiniToml.java`.

## Contents

- [On a computer](#on-a-computer)
- [app.toml](#apptoml)
- [Package format](#package-format)
- [Repositories](#repositories)
- [pkg](#pkg)
- [Identity](#identity)
- [Permissions](#permissions)
- [Where permissions are enforced](#where-permissions-are-enforced)
- [Saved state](#saved-state)
- [Background (ePhone)](#background-ephone)
- [For the compositor and the phone](#for-the-compositor-and-the-phone)
- [app_ctl](#app_ctl)
- [System apps](#system-apps)
- [App Store](#app-store)
- [File picker and chooser](#file-picker-and-chooser)
- [Settings → Apps](#settings--apps)
- [Sample repository and scenario](#sample-repository-and-scenario)
- [Discord](#discord)
- [Integration with the GUI](#integration-with-the-gui)
- [Tests](#tests)
- [Limits and open items](#limits-and-open-items)

## On a computer

| Path | What |
| --- | --- |
| `/apps/<id>/` | An installed app: `app.toml`, the entry, assets, `.pkg-origin` (where it came from and its trust) |
| `/var/apps/<id>/` | The app's own data, always readable and writable by it. `state` holds its last `SaveState` |
| `/var/lib/apps/db.json` | Permission answers, one-file grants, "open with" defaults. No app can reach `/var/lib/apps` |
| `/var/lib/apps/session` | Apps running at the last shutdown |
| `/etc/pkg/repos` | Repositories: `<name> <url> [<pinned key>]` per line |
| `/var/lib/pkg/<repo>.index.json` | Cached indexes |
| `/etc/pkg/signing.key` | A publisher's signing key (`pkg keygen`) |
| `/home/` | Shared documents (`files:documents`) |

## app.toml

```toml
id          = "dev.evan.notes"         # required, reverse-DNS
name        = "Notes"                  # required, 1..64 chars
version     = "1.2.0"                  # required, MAJOR.MINOR.PATCH[-pre|+build]
entry       = "main.py"                # required, relative path to a .wasm or .py
summary     = "Write things down"      # optional, <= 256 chars
publisher   = "Evan"                   # optional, <= 64 chars
sizes       = ["compact", "medium", "wide"]           # optional, default all three
permissions = ["files:documents", "notifications", "network"]
background  = { refresh = true, modes = ["audio"] }   # optional
open_with   = [".txt", ".md"]          # optional, lower-case extensions
hotkeys     = [{ slot = 1, action = "new_note", label = "New note" }]   # slots 1..6, unique
icon        = "icon.png"               # optional, relative .png that must be in the package
```

- **id**: two or more dot-separated labels of `[a-z0-9_-]`, the first starting with a letter, at most 128 characters (the wire limit of `app_id`).
- **Strict.** Unknown keys are errors, at the top level, in `background` and in hotkeys (like `.ui.json` layouts). A newer key means a newer `pkg`.
- **Background wishes declare their permissions.** `refresh = true` adds `background:refresh`, each mode adds `background:<mode>` to the declared permissions.
- **Declared = the most an app can ever get.** A permission that isn't declared is denied without asking.
- **Versions** compare numerically by MAJOR.MINOR.PATCH; a pre-release (`-beta`) sorts before its release.
- **Errors** name a field: `syntax`, `unknown`, or the key (`id`, `entry`, `permissions`, …). The vector files are named after it: `bad_<field>_<n>.toml`.

### TOML subset

`key = value` lines, `#` comments, one level of `[table]` headers, basic strings (`"…"` with `\" \\ \n \t \r \uXXXX`), literal strings (`'…'`), decimal integers (`_` allowed), booleans, arrays (may span lines, trailing comma allowed) and inline tables. Dotted keys, nested headers, `[[arrays of tables]]`, multi-line strings, floats and dates are syntax errors, so both parsers stay small and identical.

## Package format

An `.ecmpkg` is an **uncompressed POSIX ustar archive** (any `tar` can list it):

- the first member is `app.toml`;
- only regular files (typeflag `0`/NUL) and directories (`5`); no links or devices;
- relative paths without `..`, `.`, `\` or empty parts (names up to 100 bytes plus the 155-byte ustar prefix);
- at most 512 entries and 8 MiB unpacked;
- the entry file (and the icon, if named) must be in it.

`pkg build <dir>` writes `<id>-<version>.ecmpkg` with `app.toml` first and the other files in sorted order, skipping dot-files.

## Repositories

A repository is a plain HTTP server with an `index.json` next to the package files. `pkg serve <dir> [port]` is one (any computer can run it); any HTTP server that serves the folder works.

```json
{
  "format": "ecm-repo-1",
  "name": "/srv/repo",
  "publisher_key": "<64 hex: ed25519 public key>",
  "packages": [
    { "id": "dev.ecm.hello", "name": "Hello", "version": "1.0.0",
      "summary": "Says hello", "permissions": ["network"],
      "file": "dev.ecm.hello-1.0.0.ecmpkg", "size": 3072,
      "sha256": "<64 hex>", "signature": "<128 hex>" }
  ]
}
```

`publisher_key` and `signature` are optional, and so are three fields the App Store shows: `publisher` (from `app.toml`), `icon` (the icon's path in the package) and `screenshots` (PNG members under `screenshots/` in the package, sorted, at most 4). A repository serves those members at `<file>/<member>` (e.g. `dev.ecm.notes-1.0.0.ecmpkg/screenshots/1.png`), extracted from the package; `pkg serve` does, and only for members the index lists. `file` must be a plain file name; icon and screenshots must be safe relative `.png` paths (screenshots under `screenshots/`). Clients take the newest version per id. Older indexes without these fields still parse.

### Signing and trust

- A signature is ed25519 over `"ecmpkg-sig-v1\n" + id + "\n" + version + "\n" + sha256 + "\n"` (binds the metadata to the archive hash). Keys come from `ecm-ssh-crypto`.
- `pkg keygen` makes a key; `pkg build <dir> <out> --sign <key>` writes `<file>.sig` and `publisher.pub` into `<out>`; `pkg serve` puts them into the index.
- Every download is checked against `size` and `sha256`. Then:

| Repository | Package | Result |
| --- | --- | --- |
| key pinned (`pkg repo add … --key <hex>`) | valid signature | **verified publisher** |
| key pinned | unsigned or bad signature | refused |
| no pinned key | signed (valid against `publisher_key`) | unverified publisher (signed, key not pinned) |
| no pinned key | unsigned | **unverified publisher** |

## pkg

```text
pkg repo add <name> <url> [--key <hex>]   pkg repo list | remove <name>
pkg update                                refresh indexes, upgrade installed apps
pkg search [term]                         pkg install <id> | <file.ecmpkg> [--force]
pkg remove <id> [--purge]                 pkg list | info <id>
pkg run <id> [args]                       launch an app, show its output and any denials
pkg perm list [id] | grant|deny|reset <id> <permission>
pkg open <path> [--with <id>]             open a file with its default app (or the named one; .vpl / .gba: the built-in program)
pkg build <dir> [out] [--sign <key>]      pkg keygen [file]
pkg serve <dir> [port]                    default port 8080
```

The repository logic (repository list, cached and fetched indexes, the catalog across repositories, install / update / remove planning, downloads with progress, unpacking and `.pkg-origin`) is the library module `ecm_pkg::client`, used by `pkg` and the App Store alike; tests run it against an in-memory repository (`MemoryNet`).

Installing unpacks into `/apps/<id>/` (replacing an older version), creates `/var/apps/<id>/` and tells the app manager to reindex through [`app_ctl`](#app_ctl) (a host call, not a file watch: it is immediate and needs no polling). `pkg` itself is a normal program typed in the terminal, so it is unconfined.

## Identity

- **An app is a process group.** `AppManager.launch(id, args)` registers the group, then spawns the entry into it (`main.wasm` directly; `main.py` as `python /apps/<id>/main.py`), so the very first instruction already runs as the app. Environment: `COLUMNS`, `LINES`, `ECM_APP_ID`, `ECM_APP_DATA`.
- **Children inherit it.** Child programs are started by the kernel. When the kernel spawns a program while it is handling a request from process P (P typed a line into its shell session, as sshd does), `ComputerInstance.process_spawn` puts the new program into P's app group (`NetIpcBridge.dispatchingPid()` → `AppManager.groupForSpawn`). No ABI change was needed: the kernel only spawns from inside such a request or from its own console.
- **Terminal commands are not apps.** Programs typed in the terminal (or started by unconfined programs) are in no app group and are unconfined, exactly as before.
- `appIdOf(pid)` / `instanceOf(pid)` work for every member; an instance lives until all its members have exited.

## Permissions

Names from [UI Protocol §10](UI_PROTOCOL.md#10--apps-and-permissions), plus `packages` (install, launch and manage apps through `app_ctl`; for the App Store):

`network`, `radio`, `cellular`, `controller`, `microphone`, `location`, `notifications`, `files:documents`, `files:all`, `packages`, `mixer` (change the [Volume Mixer](#volume-mixer): other apps' volumes and speakers), `contacts` (the address book, `/home/.contacts`), `phone` (calls and everything else of `telephonyd`'s, its port 7720), `sms` (texts only, port 7726), `peripherals:<type>`, `background:refresh|audio|call|radio|transfer|navigation`.

How a check is decided:

1. Not declared in `app.toml` → denied, never asked.
2. A remembered answer (`/var/lib/apps/db.json`) → that answer.
3. An answer for this run only (`remember = false`) → that answer.
4. `background:*` → never asked mid-call: `background:refresh` is switched on in Settings > Apps; `background:<mode>` is asked once, without blocking the app, the first time the app is hidden while that mode is on ([Background](#background-ephone)), and the answer is remembered.
5. Otherwise ask the `PermissionPrompter`. Concurrent asks for the same app and permission share one prompt. No answer within `apps.promptTimeoutSeconds` (60) → denied, not remembered.
6. **No prompter** (console mode, nothing can ask): denied for this run and logged with the `pkg perm grant` command to allow it, unless the server sets `apps.allowWithoutPrompter = true` (then allowed for this run).

Denials are logged once per app run (`[apps] <id> denied <perm>: <why>`) and kept on the `AppInstance` (`pkg run` prints them). Settings later: `PermissionStore.decisions()`, `setDecision`, `reset`, `grants`, `revokeGrant`, `defaultHandlers`.

### Files

A confined app may:

- read and write `/var/apps/<id>/**` (its own data) — always;
- read `/apps/<id>/**` (its own code), never write it;
- open a file it was granted one at a time (file picker, open with): exact path, read or read-write;
- with `files:documents`: `/home/**`;
- with `files:all`: the whole disk except other apps' data (`/var/apps/*`), writing to `/apps`, and the app database;
- with `packages` (the App Store): what `pkg` touches — `/apps/**`, `/etc/pkg/**` (never `/etc/pkg/signing.key`) and `/var/lib/pkg/**`;
- read (never write) public system configuration, `AppManager.PUBLIC_CONFIG`: `/etc/chat.conf` (the Tech Village Discord backend, which Discord connects to); nothing secret is ever added there;
- never touch `/var/lib/apps`.

A forbidden read looks like a missing file (`NOENT`), so nothing leaks; a forbidden write of an existing file is `ACCES`.

## Where permissions are enforced

In Java, at the host-call boundary, through `ProcessSandbox` (implemented by `AppManager`, installed with `ProcessManager.setSandbox`). Unconfined pids pass every check.

| Host calls (child ABI) | Permission | Denied result |
| --- | --- | --- |
| `sock_socket` (TCP/UDP, DNS, netlink, packet sockets) | `network` | -13 |
| `sock_socket(AF_ECM_TUN)` (radio0) | `radio` | -13 |
| `wifi_ctl` (wlan0: iw, wpa_supplicant) | `radio` | status -13 |
| `net_set_promiscuous_on`, `net_pcap_enable` | `radio` | -13 |
| `periph_call`, `periph_methods` | by type: `sdr amplifier antenna dish microwave_radio wifi` → `radio`; `lte_modem mmwave_modem` → `cellular`; `xbox_controller controller_receiver` → `controller`; `microphone` → `microphone` (also `/dev/mic*`); else `peripherals:<type>` | error frame "permission denied" |
| `periph_wait_event` | only events of peripherals already allowed (filtered when posted) | — |
| `redstone_*` | `peripherals:redstone` | -13 / 0 |
| `/dev/sdr*` (`path_open`) | `radio` | `ACCES` |
| `/dev/audio*` | `peripherals:speaker` | `ACCES` |
| `path_open`, `path_filestat_get`, `path_create_directory`, `path_unlink_file`, `path_remove_directory`, `file_*`, `video_open` | the file rules above | `NOENT`/`ACCES`, -13 |
| `app_ctl` (anything but `whoami`) | `packages` | -13 |
| `sock_connect` to 127.0.0.1:7720 / 7726 (`telephonyd`) | `phone` (7720) / `sms` (7726); then each `dial`, `sms`, `mms` a non-system app sends is confirmed by the player (`confirm:<call\|sms\|mms>:<to>`, [TELEPHONY.md](../TELEPHONY.md#permissions)) | -13; `err` reply when declined |
| `/home/.contacts` | `contacts` | `ACCES` |
| `/var/telephony` | never (telephonyd's) | `ACCES` |

Every socket path starts with `sock_socket`, so the other `sock_*` calls need no check. `location` and `notifications` belong to services that don't exist yet; their owners call `AppManager.check(pid, perm)`. `path_remove_directory` (empty directories) was implemented on the way, since `pkg remove` needs it.

## Saved state

`WillSuspend` → `SaveState` → `Restore` (UI Protocol §11), stored in `/var/apps/<id>/state` (written atomically, at most 256 KiB).

- **Terminals at shutdown and reboot (PC policy).** `TerminalBlockEntity.shutdownComputer` calls `ComputerInstance.shutdownApps()` before interrupting the computer (`close()` does it too, idempotently). `AppManager.shutdown()` records the running apps (`/var/lib/apps/session`), sends `WillSuspend{deadline}` to each through `AppEvents.willSuspend`, waits until every app that got it has answered or the deadline (`apps.saveStateDeadlineMs`, 500) passes — one deadline for all — and stops the apps. An app that misses the deadline keeps its previous state.
- **Next launch.** When the app's connection says `Hello`, the compositor sets `restore_follows = peekRestore(id)` and sends `takeRestore(id)` as `Restore` right after `Welcome`. The file stays until the app saves again, so a crash right after a restore loses nothing. `lastSession()` lists what was running (a desktop may relaunch it).
- **Without a compositor** nothing has a UI connection, `willSuspend` returns false, and shutdown doesn't wait.

## Background (ePhone)

`AppManager.setPolicy(LifecyclePolicy.PHONE)`; `BackgroundScheduler` (`apps.background()`) on the pause API (an app = a process group; suspended = group paused). The shared names are in `plans/background-apps/PLATFORM_API.md`; Discord and Music are built on it.

- **Where the phone is.** Held (main or off hand): awake. In the hotbar **or the main inventory (a pocket)**: asleep, with its radios on, refresh wakes and background modes running. In a chest, on the ground, in a dock, with an empty battery or with its holder logged out: off.
- **Suspend / resume.** `suspend(app)` collects its state with the deadline, then pauses the group. `resume(app)` resumes it (and ends a refresh window it was in).
- **Refresh wakes.** For a suspended app that asks for `refresh` and holds `background:refresh`: every `apps.refreshIntervalMinutes` (5) the group is resumed for at most `apps.refreshBudgetMs` (2000) of wall time, then paused again. If the computer sleeps, it is woken for the window with `ComputerInstance.wakeFor` (installed as the scheduler's `SleepControl`). `BackgroundDone` (`onBackgroundDone(pid)`) ends the window early. One window at a time; due apps wait for the next `tick`. Drive it with `tick(nowMs)` from the phone's server tick, or `start()` for an own 1 s timer. `refreshNow(app)` starts a window at once and returns it (`isOver`, `endedBy` = `budget` / `done` / `resumed`, `ranMs`, `reason`).
- **Background modes.** `audio`, `call` (microphone, speaker and network: a voice call), `radio`, `transfer`, `navigation`, declared in `app.toml` (`background = { modes = [...] }`). An app starts or stops one with the UI message `BackgroundMode{mode, on}` (Rust `Core::background_mode`, Python/Lua `app.background_mode(mode, on)`); writing to the phone's speaker also starts `audio` by itself (it ends 2 s after the last write, unless the app turned it on itself). While a mode is on and `background:<mode>` is allowed, a hidden or asleep app is duty-cycled: resumed for its share of every 100 ms, `apps.backgroundCpuPercent.<mode>` (10 %; `call` 25 %; with several modes, the largest). There is no per-process CPU accounting, so the cap is wall-clock run time per group. `activeModes()` (app id → modes) feeds the status bar (`AUDIO`, `CALL`, ...); `AppEvents.backgroundModesChanged` reports changes and the shell gets `AppModes{pid, app_id, modes}`.
- **The first-time prompt.** The first time an app is hidden (or the phone goes to sleep) while a mode with an undecided `background:<mode>` is on, the shell asks once: "Keep Discord running in the background for calls?" (Allow / Don't allow, remembered as `background:<mode>`). The app is not blocked meanwhile; it gets `BackgroundModeResult{mode, granted}` once the answer is known (at once when it was already decided). Don't allow stops the mode; the app stays suspended. Settings > Apps changes the answer later.
- **The limit.** At most `apps.maxBackgroundApps` (2) apps may have modes on at once. When one more starts, the app whose mode started longest ago (never the one on screen) has its modes stopped and stays suspended, and a notification names it: "Music was paused to let Discord run" (tapping it opens Music).
- **Wake sockets.** A socket with `SO_ECM_WAKE` (`setsockopt(fd, SOL_SOCKET, SO_ECM_WAKE, 1)`; Rust `ecm_host_abi::socket::set_wake`, Python/Lua `net.set_wake(conn, true)`) wakes its suspended app as soon as data (or a connection, or a close) arrives: the kernel reports it with its `socket_wake` import as soon as the socket becomes readable, and again every 2 s while nobody reads it (the first one may have come before the app was allowed to run), and `wakeNow(app, "socket")` gives the app a refresh window right away (it needs `background:refresh`; at most once per `apps.wakeMinIntervalMs`, 2 s, per app; a wake that comes while another app's window is open follows it). A sleeping phone with wake sockets lets each network interrupt run the kernel for 60 ms (at most every 250 ms), like a Wi-Fi chip's wake-on-packet, so a message for a pocketed phone arrives in seconds, not at the next 5-minute refresh. Java callers outside sockets (a modem page) use `BackgroundScheduler.wakeNow(app, reason)` or `ComputerInstance.wakeFor`.
- **Wake locks.** A ringing or running call has to keep a phone in the hotbar or a pocket running, not just give it slices. `/dev/uictl wakelock <ms>` (the `gui wakelock` command; Rust `ecm_ui::wakelock(ms)`, Python and Lua `ui.wakelock(ms)`) keeps the computer awake for `<ms>` from now, at most `apps.wakelockMaxMs` (10000) per renewal; the holder sends it again before it runs out, and `wakelock 0` releases it. Locks are per program and released when it exits; `wakelock` with no argument lists the holders (`<owner> <ms left>`). Programs outside the app sandbox (daemons such as telephonyd) may hold one freely; an app only while its `call` or `audio` mode is on (otherwise `gui: wakelock needs an active call or audio mode`). In Java, `ComputerInstance.acquireWakeLock(owner, ms)` returns a handle with `renew(ms)`, `release()` and `isHeld()`; the scheduler's duty-cycle slices and refresh windows are wake locks too, so one ending can't put to sleep a computer a call holds awake. A phone switched off (a chest) sleeps regardless (`sleepIgnoringWakeLocks`). Discord holds one for the length of a voice call, renewed every second of its own run time. The app's own group is still duty-cycled by its mode's CPU share; the lock keeps the kernel, the network and daemons running.
- **MIC.** While a hidden app (or any app of an asleep phone) reads `/dev/mic`, the status bar shows `MIC` (`PhoneUi.micActivity`; `/dev/uictl phone` lists `mic <app>`).
- **Media sessions.** An app that plays sends `MediaSession{title, artist, album, art, state, position_ms, duration_ms, actions}` (Rust `Core::media_session`, Python/Lua `app.media_session(...)`); the shell gets `MediaSessionChanged` and shows a Now Playing card in the notification sheet, whose buttons send `MediaControl{pid, action}`, which the app receives as `MediaAction{action, position_ms}` (`App::on_media`). A double-tap on worn AirPods (the "Tap AirPods" key pressed twice) sends a Bluetooth button frame to their phone, which plays or pauses the active session.
- **Wasmtime limit** ([UI Protocol §15](UI_PROTOCOL.md#pausing-what-works)): a group in a pure compute loop only stops at its next host call there, so budgets and duty cycles are approximate for such apps; `ProcessManager.awaitHalted` tells whether a pause has landed.

### On a Terminal (the desktop)

`LifecyclePolicy.PC` keeps every app running when minimized, so modes there only tell the desktop: closing the window of an app with an active mode hides it to a tray icon in the taskbar instead of quitting it (click the icon to bring it back; right-click > Quit to quit, which ends a call). The tray also has the Now Playing entry and a speaker icon that opens the Volume Mixer. At its first start (not after a shell restart) the desktop relaunches the apps that were running at the last shutdown (`lastSession()`).

## Volume Mixer

Every speaker is a mixer (`SpeakerAudio`): each open of `/dev/audio` is its own stream with its own format, clock and **role** (`/dev/audioctl role media|voice|phone`; Rust `ecm_audio::Device::set_role`, Python/Lua `speaker.set_role("voice")`). The mixer resamples the streams to one rate and mixes them; while a stream carries sound (louder than -50 dBFS over the tick) every stream with a lower role (`media < voice < phone`) is ducked to `audio.duckLevelPercent` (25 %) with a 150 ms fade and comes back 400 ms after it goes quiet, so music dips only while someone in a Discord call talks. Reads of `/dev/audioctl` report the stream's `role` and `ducked`. **Timing (display pipeline).** The mixer is upstream of `SpeakerAudio.drain()`, the one output point that the display pump drains about every 20 ms: each drain is stamped with the mixer's own schedule after mixing and ducking (`lastDrainPtsUs()`, display-clock microseconds), so the ducking fades sit on the same timeline as the samples they shape, and a drain that a `voice` or `phone` stream put sound into is live (`lastDrainLive()`, `isLiveRole`; sent with `FLAG_LIVE` to play at once, not on a video's playout delay). `/dev/audioctl` also reports `pts_us`: when a sample written now would play, on the clock of `ui.now_us` and `CommitAt` (Rust `ecm_audio::Device::next_pts_us`, Python/Lua `speaker.status()["pts_us"]`), so a program can commit pictures at the time its sound plays. Seek and pause in Music flush the stream's queue (`flush`); the Video Player's sound is an ordinary `media` stream in the mixer.

On top of that, each computer has a Volume Mixer, `/dev/mixer`: one strip per app that plays (volume 0-150 %, mute, a live peak meter, the "ducked" badge, the speaker it plays on and, on a Terminal with several speakers, a route to another one), a master strip per speaker (volume 0-100, mute; on a phone it is the ePhone speaker, or AirPods while they play, each with its own volume) and a strip per microphone (gain, level). App settings are kept per app id in `/var/lib/apps/mixer.conf`, the app database no app can write. It is shown in Settings > Sound > Mixer, the phone's notification sheet (a volume slider that expands into the strips), the desktop tray's speaker popup and the `mixer` command (`man mixer`); Python and Lua have `audio.mixer()`. Anyone may read it; writing needs the `mixer` permission for apps (Settings has it).

```
/dev/mixer, read:   output left volume=100 mute=0 peak=1200 label=Speaker
                    app dev.ecm.music volume=80 mute=0 route=- on=left peak=5000 ducked=1 role=media streams=1
                    input top gain=50 level=-42
/dev/mixer, write:  set <app|master[.side]> <percent>   mute <target> [on|off|toggle]   unmute <target>
                    route <app> <side|->                 gain <input> <0-100>            reset <app>
```

## For the compositor and the phone

`computer.getAppManager()` (one per computer). The compositor implements two small interfaces and calls a handful of methods; nothing else in `computer/apps` needs touching.

**`PermissionPrompter`** (install with `setPrompter`; null = console mode)

- `CompletableFuture<Answer> prompt(Request request)` — `Request(req, appId, appName, permission, pid)`, `Answer(allow, remember)`. Called on the app's thread, which waits; return a future (send `PermissionPrompt`, complete it from `AnswerPermission`), or null when no shell can show it.

**`AppEvents`** (install with `setAppEvents`; every method has a no-op default)

- `boolean willSuspend(AppInstance app, int deadlineMs)` — send `WillSuspend` to the app's connections; false if it has none.
- `void openFile(AppInstance app, String path)` — send `OpenFile` (the grant is already in place).
- `void refreshWake(AppInstance app, long budgetMs)` — optional hint that a refresh wake started.
- `void backgroundModesChanged(AppInstance app, Set<String> modes)` — status-bar indicators.

**Calls into the app manager**

| Call | When |
| --- | --- |
| `appIdOf(pid)`, `instanceOf(pid)`, `manifest(id)`, `installedApps()`, `running()` | Hello / identity, launcher, switcher |
| `launch(id, args)` → `AppInstance` | launcher |
| `check(pid, permission)` | services that need a permission (`notifications`, `location`); may prompt |
| `onSaveState(pid, bytes)` | `SaveState` arrived |
| `peekRestore(id)`, `takeRestore(id)` | `restore_follows` and `Restore` after `Welcome` |
| `onBackgroundDone(pid)` | `BackgroundDone` arrived |
| `onFilePicked(pid, path, write)` | the picker answered (`FilePicked`): one-file grant |
| `open(path, fromPid)` | `Open{path}` from Files or an app; throws `AppException.CHOOSE` with the candidates when there are several handlers and no default |
| `openWith(path, id)`, `handlersFor(path)`, `defaultHandlerFor(path)`, `setDefaultHandler(ext, id)` | the chooser and Settings |
| `permissions()` (`PermissionStore`) | Settings: list, change, revoke answers and grants |
| `setPolicy(PHONE)`, `suspend(app)`, `resume(app)`, `background()` | the ePhone host |

### Wired so far (P5b, ePhone; merged with the integration)

For every computer with the GUI (Terminal and ePhone): `SaveState` -> `onSaveState` (a non-app's
`SaveState` is dropped and logged), `restore_follows` / `Restore` right after `Welcome` (`peekRestore` /
`takeRestore`), then queued notification actions, then queued `OpenFile`s; `BackgroundDone` ->
`onBackgroundDone`; the app id in `Welcome` / `WindowOpened` from `UiManager.appId` (`appIdOf`, else
the program name); all through `CompositorHost.restoreFor/saveState/backgroundDone/appId` implemented by
`UiManager`. `UiManager` also owns the shared delivery both kinds of computer use: `promptViaShell`
(the one prompter path: the shell's dialog through `Compositor.requestPermission`, remembered answers
stored even after a prompt timeout), `deliverWillSuspend` (`WillSuspend` through `Compositor.sendToApp`)
and `deliverOpenFile`. On Terminals `installAppHooks` installs a prompter and `AppEvents` (`willSuspend`,
`openFile`) built on them; on a phone `PhoneUi` is the `PermissionPrompter` (delegating to
`promptViaShell`) and the `AppEvents` (`willSuspend`, `openFile`, plus the phone's `refreshWake` /
`backgroundModesChanged`), `setPolicy(PHONE)` and the scheduler's own timer (`background().start()`).
`Launch` of an installed app goes through `UiManager.launch` on both; on a phone it calls
`PhoneUi.launch` (one instance per app, the session list kept). The phone's built-in apps
(`dev.ecm.notes`, `dev.ecm.gba`, `PhoneUi.provision`, then `reindex`) sit beside the system apps
(`SystemApps`: `dev.ecm.store`, `dev.ecm.editor`, provisioned by every `reindex`) in `/apps`; their folders
differ, so neither installer touches the other's. See [EPHONE.md](EPHONE.md#the-phone-gui-p5b).

## app_ctl

The child import behind `pkg` (`abi/child-abi.toml` `[imports.apps]`, Rust `ecm_host_abi::apps`): `app_ctl(req, len, reply, cap, status_out) -> full reply length`. Requests are words separated by `\n`:

`whoami` · `reindex` · `list` · `ps` · `launch <id> [args…]` (→ pid) · `status <pid>` (`running` / `suspended` / `exited <code>`, then `denied <perm>` lines) · `output <pid>` (buffered output of the app's main process, up to `cap` bytes, taken once) · `kill <pid>` · `open <path>` · `default <.ext> <id|->` · `session` · `perm list [id]` · `perm grant|deny|reset <id> <permission>`.

Status: 0 ok, -2 not found, -13 not permitted, -22 bad request, -35 choose an app. Confined processes may only use `whoami` unless they hold `packages`. The simulator has no app manager (-1).

For the App Store, the chooser and Settings:

| Request | Reply |
| --- | --- |
| `info <id>` | `key<TAB>value` lines: `name`, `version`, `publisher`, `summary`, `permissions` (space-separated), `open_with`, `trust` (from `.pkg-origin`; `system app` without one), `icon`; -2 if not installed |
| `handlers <path>` | `<id><TAB><name><TAB>default` (or `-`) per app that opens the file's type (`handlersFor`) |
| `defaults` | `<.ext><TAB><id>` per open-with default |
| `openwith <path> <id>` | `<id> <pid>`: `AppManager.openWith` (one-file read-write grant, launch, `OpenFile`); a confined caller must be able to read the file |
| `perm revoke <id> <path>` | removes a one-file grant; -2 if there is none |

Parsers and an in-memory app manager with the same replies: `rust/crates/ecm-apps-ui/src/appctl.rs` (`FakeCtl`).

## System apps

`SystemApps` (Java) installs apps every computer has into `/apps/<id>/` when the app manager indexes (boot, every `reindex`): the **App Store**, `dev.ecm.store` (program `store` as `main.wasm`, `icon.png` from the mod's resources, permissions `packages` and `network`), the **Text Editor**, `dev.ecm.editor` (program `editor`, no permissions, `open_with` the text types `.txt .md .py .json .toml .sh .rs …`: what Files and `Open` open text with, confined to the file it was given), the **Video Player**, `dev.ecm.video` (program `videoplayer`, permissions `files:documents` and `peripherals:speaker`, `open_with` the video types; [below](#video-player)), and the **Browser**, `dev.ecm.browser` (program `browser`, permissions `network` and `files:documents`; [BROWSER.md](BROWSER.md)). On a first install the declared permissions are allowed once (remembered, revocable in Settings or with `pkg perm`). It is copied again when the program's size or the manifest changes; `pkg remove dev.ecm.store` lasts until the next boot. Service apps are installed only where the service runs: **Cell Site** (`dev.ecm.cellsite`) on computers with `/etc/celld-site.toml`, **Carrier Console** (`dev.ecm.carrier`) on computers with `/etc/cored.db` or `/etc/cored.toml` (both `network`, for their daemon's loopback admin socket; [CELLULAR_V2.md](CELLULAR_V2.md#as-built-phase-3-network-side)). The **Card Designer** (`dev.ecm.carddesigner`, [../cards/CARDS.md](../cards/CARDS.md)) is the first Python system app: its `app.toml`, `main.py` and icon ship in the mod's resources (`ecm_apps/dev.ecm.carddesigner`), `SystemApps` copies them in (`App.python`) wherever the `python` program exists, and the app manager runs `python main.py`; permission `peripherals:card_printer`.

## Video Player

`videoplayer` (`rust/wasm-programs/videoplayer`; screens, clock and sound pacing in its `lib.rs`, tested on the host with `cargo test -p videoplayer`), the system app `dev.ecm.video` (permissions `files:documents` and `peripherals:speaker`, `open_with` `.mp4 .m4v .mkv .webm .mov .avi`). It is installed only on servers with the optional, server-side **`evanscomputermod-ffmpeg`** sidecar (the `VideoBackend` that `api/video/VideoBackends` finds with ServiceLoader; the main mod has no native code), and `SystemApps` removes its copy when the sidecar is gone. Without it `video_supported` is 0, `video_open` returns -4 and `video`/`player` say video isn't installed. One window, the same on a desktop and an ePhone:

- **Library**: the video files in `/home` and `/home/videos` with their sizes (Refresh; Prev/Next pages when they don't fit; the wheel scrolls). A tap plays one. Files' double-click on a video opens it here (`OpenFile`, a one-file grant).
- **Playing**: the picture in a `canvas` node, fitted to the window with its aspect ratio. The host decodes it with FFmpeg straight into RGB565 at the picture's size (`video_decode_frame`); above 400x240 it decodes smaller and the canvas scales it up by a whole number. Under it: the seek bar (a click jumps there), `m:ss / m:ss`, whether there is sound (`sound`, `no sound`, `no speaker`), and -10s, Play/Pause, +10s and Loop (Loop only where it fits). Keys: Space or K play/pause, Left/Right 10 s, L loop, Esc or Backspace back to the library.
- **Timing**: a wall clock is the master; a frame shows when its time comes, late frames are dropped (up to 8 in a row) rather than slowing the clock. Sound is read from the host (`video_audio_open` at 22050 Hz mono, `video_read_audio`) and kept 200 ms ahead of the clock on the speaker (`/dev/audio`, 400 ms latency); sound that falls behind (a stall) is skipped, not played late. Pause and every seek flush the speaker.
- **Lifecycle**: `WillSuspend` pauses it and saves `{"path", "pos"}`; after a cold start `Restore` opens the same file there, paused. On an ePhone it claims hotkey slots 1 (Play/Pause), 2 (Back 10s) and 3 (Forward 10s).
- **Resizing** the window to a size that needs another decode size reopens the decoder at the same position.

The console command `video` (`rust/wasm-programs/video`) shares the clock and sound pacing: `video info`, `video ls`, `video play [--screen] [--size WxH] [--start S] [--loop] [--mute]` (frames go straight to the Terminal's graphics or the Screen wall with `video_decode_to_gfx`, in RGB565).

## App Store

`store` (`rust/wasm-programs/store`, logic in `rust/crates/ecm-apps-ui/src/store/`): an `ecm-ui` app over `ecm_pkg::client` (the same code as `pkg`, no shelling out) and `app_ctl`.

- **Discover**: every app across the configured repositories (newest version wins), with search over id, name, summary and publisher. **Updates**: installed apps with a newer version (and *Update all*). **Installed**: what is in `/apps`.
- **An app's page**: icon, name, publisher with a badge — **Verified publisher** (signed with the repository's pinned key), **Unverified publisher (signed, key not pinned)**, **Unverified publisher** (unsigned); a pinned repository with an unsigned package shows *Unsigned* and can't be installed — version, size, repository, summary, the permissions it asks for in plain words (`perms::describe`; for installed apps with the current answer: Allowed / Denied / Asks first), and screenshots. Buttons: Install / Update / Open, and Remove (keeps `/var/apps/<id>`; system apps can't be removed).
- **Progress**: installs, updates and removes are queued; a download redraws the progress bar every few percent (`Installing Notes  12.0 KB of 30.0 KB`); the message line says what happened (`Installed Net Check 1.0.0 (unverified publisher)`). After every change the app manager reindexes through `app_ctl`; Open launches through `app_ctl launch`.
- **Pictures**: icons (installed apps' from their folder, others' from the repository) and screenshots are decoded in the app (`png.rs`, 8-bit PNGs up to 512x512) and uploaded as raw RGBA, so a broken picture from a stranger's repository only loses that picture (the compositor closes a connection that uploads an unreadable PNG).
- **Layouts**: compact (phones, < 400 lu): tabs, one app per row, the page replaces the list; medium: tabs, cards two to a row; wide (>= 800 lu): a sidebar (Discover / Updates / Installed, the repositories, Refresh), cards three to a row and the open app's page beside them. Golden images: `store_compact*.png`, `store_medium*.png`, `store_wide*.png` in `src/test/resources/ui-golden/`.
- **A repository that is down**: its saved index (`/var/lib/pkg/<repo>.index.json`) is shown and the message says so.
- **Found on the network** ([DISCOVERY.md](../DISCOVERY.md)): opening the store and Refresh search the network for repositories (`pkg serve` on a computer running `disco serve`, or the Tech Village ISP's answer). One that isn't configured is used as if it were: its apps are in Discover and installable, the status line says "(N found on the network)", the wide sidebar and the Repositories page list it ("On the network", **Add** keeps it). Nothing is written to `/etc/pkg/repos` until Add, and a key from the network is never pinned.
- **Repositories page** (Repos in the header at compact, Repositories at medium, Add repository in the wide sidebar): the configured repositories with Remove (`pkg repo remove`), and Add by URL (`pkg repo add`, then every index is fetched; the message says `Added <name> (<url>): N apps`). `http://` is added when missing; an empty name is made from the host (`10.0.8.1` -> `10-0-8-1`). Keys can't be pinned here (that stays `pkg repo add ... --key`), so apps added this way are unverified unless signed. A phone can add one here or with `pkg repo add` in its Terminal app.
- `store --app <id>` opens on an app's page; `store --sample-repo <dir> [--sign <key>]` writes the [sample repository](#sample-repository-and-scenario); in console mode `store` says to start the GUI.

## File picker and chooser

Both are system programs started by the compositor (unconfined, not apps), drawn by the system rather than by the app that asked (UI Protocol §12). The Java side is `computer/ui/SystemDialogs` (the implementation for a compositor file-picker hook), called by `Compositor` under its lock:

```text
PickFile{req, mode, types, suggested} from an app (pid P, window W)
  -> picker --req <token> --app <app id> --mode open|save [--types .txt,.md] [--suggested <name>]
     (its window: WindowOpened.parent = W)
  <- /dev/uictl: "picked <token> <absolute path>" | "picked <token> cancel"
  -> FilePicked{req, path} to the app ("" when cancelled) and CompositorHost.filePicked(P, path, true)
     (UiManager: AppManager.onFilePicked, a one-file read-write grant; never /var/lib/apps or another app's /var/apps)

AppManager.open(path) threw AppException.CHOOSE
  -> Compositor.startChooser(fromPid, path) -> chooser --req <token> --path <path>
  <- /dev/uictl: "chosen <token> <app id> [always]" | "chosen <token> cancel"
  -> CompositorHost.openWith(path, id, always)
     (UiManager: setDefaultHandler(.ext, id) when always, then AppManager.openWith: grant, launch, OpenFile)
```

- **Tokens** are 64-bit random hex. An answer counts only when it comes from the pid started for that token (`UiCtlFd` knows the pid that opened `/dev/uictl`), and only once; an accepted line is answered `ok`, anything else `gui: …`.
- **Cancels**: a dialog that exits without answering, or can't start, answers "" (the app is never left waiting). When the asking program exits, its dialog is killed.
- **The picker** (`rust/crates/ecm-apps-ui/src/picker.rs`): places Documents (`/home`), App data (`/var/apps/<app>`), Computer (`/`); Up; folders first, files filtered by `types`, hidden files left out; the save field checks the name (not empty, at most 64 characters, no `/` or `\`, no leading dot; the first type's extension is added when missing) and asks before replacing a file. It never lists or hands out `/var/lib/apps`, other apps' `/var/apps/<id>`, `/dev` or `/etc/pkg/signing.key`, and never saves into `/apps`, `/etc`, `/var/lib` or `/bin`. Closing its window is a cancel. Open and save both grant read-write (as desktop portals do).
- **The chooser** (`chooser.rs`): lists `app_ctl handlers <path>` (the default preselected), *Always use this app for .ext files*, Cancel / Open.
- `pkg open <path>` in the terminal can't show a chooser; on -35 it says `pick one with: pkg open <path> --with <id>` (`app_ctl openwith`).

## Settings → Apps

`ecm_apps_ui::settings::AppsSection` (state: `AppsPane`), a section for the Settings app (`ecm-apps`, P3) next to Wi-Fi, Display and the others: installed apps; for the selected one its permissions with an allow toggle each, the current answer and *Ask again* (`perm grant|deny|reset`), the files it was given (`perm revoke`), and the file types it opens by default (`default <.ext> <id|->`; the toggle is disabled when it is the only handler). API: `AppsSection::new(Box::new(HostCtl))`, `title()`, `view(cx)` / `view_with(cx, side_by_side)`, `reload()`. The Settings app shows it as its **Apps** section (`settings apps`) at every size class; Settings is a built-in program (unconfined), so its `app_ctl` calls need no permission (an app embedding the section would need `packages`).

## Sample repository and scenario

`rust/crates/ecm-apps-ui/fixtures/repo/` (icons and the Notes screenshot drawn by `assets/make_assets.py`; the screenshot is cropped from the compositor's own `notes_typed` golden):

| App | What |
| --- | --- |
| `dev.ecm.notes` Notes | the §13 Notes app in Python, with a screenshot |
| `dev.visual.counter` Counter | the Visual Programmer's counter app (`main.py` and `main.ui.json` as `visual --package` writes them) |
| `dev.ecm.netcheck` Net Check | asks for `network` and connects to 10.0.7.1:8080 (the scenario's repository), showing the result in its window |

`store --sample-repo /srv/repo` writes their packages; `pkg serve /srv/repo` hosts them. Tests use the same files in memory (`samples::memory_repo`).

`/ecm scenario spawn app_store [auto|manual|fast]` (`AppScenarios.app_store`, GameTest `ecm_apps`): computer A writes and serves the sample repository; computer B (control: `store` refuses in the console; the app manager doesn't know Net Check yet) adds it, starts the GUI, runs the App Store through the app manager (`pkg run dev.ecm.store &`), opens Net Check's page (its permission in plain words), presses **Install** (the repository logs the download; the app manager knows the app; the store says "Installed Net Check 1.0.0 (unverified publisher)"), then **Open**: Net Check asks for `network` as it starts and the desktop's dialog appears ("Allow Net Check to use network?"); **Deny** (control): its window says the network was denied and nothing is remembered. Alt+F4 closes it; the desktop's launcher (which now lists it) starts it again through the app manager (its window carries `dev.ecm.netcheck`), it asks again, **Allow**, and it reaches the repository. `/ecm scenario spawn files_open_with` (GameTest `ecm_apps`): `gui`, Files from the launcher, double-click `home`, then `hello.py`: the Text Editor app shows it; Ctrl+O on `secret.py` finds nothing.

## Discord

**Discord** (`dev.ecm.discord`, program `discord`, `rust/wasm-programs/discord`: library `discord_app`) replaced the Chat app: servers with text and voice channels on every Discord backend (`discordd`) the computer can reach, found on the network by themselves. See [DISCORD.md](../DISCORD.md) for the app, the backend, voice and the protocol.

`app.toml`: `permissions = ["network", "notifications", "microphone", "peripherals:speaker"]`, `background = { refresh = true, modes = ["audio"] }`, hotkeys 1 Send, 2 Mute, 3 Hang up, sizes compact/medium/wide, the purple Discord icon (painted in Blockbench, `models/discord-logo.bbmodel`). In the Tech Villages it is served by the ring's app repository, `Tech-Village-Apps` (not pre-installed), built with the mod by `scripts/stage-wasm.sh` (`discord --source` + `pkg build`, the `village_repo` example); see [TECH_VILLAGE_NETWORK.md §8a](../TECH_VILLAGE_NETWORK.md#8a-apps-the-tech-village-repository).

Scenarios `discord_app` and `service_discovery` (`ecm_apps`), `discord_servers` and `discord_voice` (`ecm_discord`), `cell_chat` (`ecm_cellular`).

## Integration with the GUI

Wired in `gui/integration` (the compositor and the desktop from P3, the app platform from P4):

| What | Where | How |
| --- | --- | --- |
| Permission prompts | `UiManager.installAppHooks` (on `gui` start; removed on `gui stop`; not on ePhones, whose `PhoneUi` installs its own prompter and `AppEvents`) | A `PermissionPrompter` on the computer's `AppManager`: while a shell is connected it calls `Compositor.requestPermission(appId, permission, answer)` and completes the future from `AnswerPermission`; with no compositor or no connected shell it returns null, so the console policy applies. The desktop's dialog names the app from `/apps/<id>/app.toml` ("Allow Net Check to use network?"); the log line carries the app name, id and asking pid. No answer within `apps.promptTimeoutSeconds` (`AppManager.setPromptTimeoutMs` in tests) denies for this run; the dialog stays up, and a late answer with "Remember my choice" is still stored. |
| Launch and identity | `CompositorHost.launch(fromPid, appId, args)` = `UiManager.launch` | `Launch{app_id}` from the shell (desktop launcher, session `--launch`), from apps, a notification relaunch and `gui launch <id>`: an installed app starts through `AppManager.launch` (its own group, identity and sandbox); anything else is a built-in program as before. `CompositorHost.appId(pid)` = `AppManager.appIdOf(pid)`, else the program name, so `Welcome.app_id`, `WindowOpened.app_id`, the picker's title and `gui status` name the app (`dev.ecm.netcheck`, not `python`). |
| `/dev/uictl` from apps | `UiManager.control(pid, line)` | **Decision:** an app (a confined pid) may only use `launch` (and the picker / chooser answers, which are bound to their pid and token). A program it launches that isn't an installed app is spawned **into the caller's app group** (`ComputerInstance.spawnDetached(program, args, group)`), so it keeps the app's identity and permissions; an installed app launched this way gets its own sandbox (like opening another app). `Launch{app_id}` from an app follows the same rule. `start`, `stop`, `console`, `show`, `toggle`, `status` and `prompt` answer `gui: apps may only use launch`. Unconfined callers (the terminal's `gui`, the shell) are unchanged. |
| Open with | `Compositor` `Open{path}` -> `CompositorHost.open` = `UiManager.open` -> `AppManager.openAny(path, pid)` (off the compositor's lock) | The default handler gets a one-file read-write grant, is launched with the path in argv and gets `OpenFile{path}` (`AppEvents.openFile` -> `Compositor.deliverOpenFile`: at once to a connected app, otherwise right after `Welcome`). `AppException.CHOOSE` -> `Compositor.startChooser(pid, path)`. |
| Built-in handlers | `AppManager.BUILTIN_HANDLERS`, `builtinHandlerFor`, `setBuiltinLauncher` | Types no installed app opens: `.vpl` -> `visual`, `.gba` -> `gba` (started detached; for an app, in its group). Text opens in the **Text Editor system app** `dev.ecm.editor` (`SystemApps`: program `editor`, no permissions, `open_with` the text types), so it can read and write only the file it was given; files without a type open there too. The launcher's Text Editor stays the unconfined built-in program; the desktop doesn't list `dev.ecm.editor` twice. `pkg open` uses `openAny` too. |
| Files | `rust/crates/ecm-apps/src/files.rs` `open_action` | Double-click / Enter on a file sends `Open{path}` (`Ctx::open`); only `.wasm` programs are run directly. |
| Notifications | `Compositor` `Notify` -> `CompositorHost.checkNotify` = `UiManager.checkNotify` | Apps need `notifications` (`AppManager.check(pid, "notifications")`, which may prompt; on a services thread, so the compositor's lock is never held while the player decides). Refused: dropped and logged, not a protocol error. Unconfined programs post as before. |
| Settings -> Apps | `ecm_apps::settings` (section `Apps`, `settings apps`) | `AppsSection` (`view_with`: list and app side by side only when the page beside the sidebar is >= 760 lu wide) over `app_ctl` (`HostCtl`; `settings::app_with` takes another `AppCtl`, e.g. `FakeCtl` in tests and goldens). |
| The launcher | `ecm_apps::desktop::launch_items` | The core apps (now with the **Visual Programmer**, `visual`, pixel-art icon `visual`) and the apps in `/apps` (the **App Store** with icon `store`), read again each time the launcher opens, so an app installed from the store is there at once. |

**After the ePhone merge:** `WillSuspend`, `SaveState` -> `AppManager.onSaveState` (the compositor's own map is gone), `restore_follows` / `Restore` after `Welcome` and `BackgroundDone` -> `onBackgroundDone` are wired for every computer (see "Wired so far" above); phone hotkeys (`HotkeyClaim`) and `refreshWake` / `backgroundModesChanged` are the phone's (`PhoneUi`). Still open: relaunching `lastSession()` on the desktop and status-bar indicators for background modes on Terminals.

## Tests

| What | Where | Run |
| --- | --- | --- |
| TOML subset, manifest, archives, index, trust, HTTP, repo list (Rust) | `rust/wasm-programs/pkg/src/*` | `-Rust pkg` |
| Java manifest on the shared vectors | `AppManifestVectorsTest` | `-JUnit AppManifestVectorsTest -McVersion 26.1` |
| Deny prompt and remembered answer; undeclared and console-mode denials; child inherits the app's permissions; open-with grant allows only that file (plus own data, own code read-only, others' data and the app DB never); state survives a restart; refresh budget, BackgroundDone, no-permission control, exception mode | `AppPlatformTest` (real kernel, `apptest` probe) | `-JUnit AppPlatformTest -McVersion 26.1` |
| Computer A serves a repository, B installs over a cable with `pkg`, network denied until granted | `ecm_apps` (`AppTests`, `AppScenarios.app_repo`) | `-GameTests ecm_apps` |
| Repository client: catalog across repositories, install / update / remove planning, progress, trust and pinning, tampering, unreachable repositories, screenshots | `rust/wasm-programs/pkg/src/client.rs` | `-Rust pkg` |
| Store logic and window (cards, pages, planning, badges, install / open / remove through the window, screenshots, sizes), picker (args, path safety, names, filters, overwrite), chooser, Settings section, app_ctl parsers, PNG decoder, sample repository | `rust/crates/ecm-apps-ui` | `-Rust ecm-apps-ui` |
| Store at compact / medium / wide (list and page), picker, chooser: golden images | `UiGoldenTest` (sessions: `cd rust; cargo run -p ecm-apps-ui --example record_store_sessions -- ../src/test/resources/ui-sessions`) | `-JUnit UiGoldenTest -McVersion 26.1` |
| `PickFile` -> picker -> `picked` -> `FilePicked` + grant; wrong pid, wrong token and a second answer refused; cancel, exit and a missing picker answer ""; window attached; killed with its app; chooser -> `openWith` | `UiSystemDialogsTest` | `-JUnit UiSystemDialogsTest -McVersion 26.1` |
| The system App Store and its permissions; `info`, `handlers`, `defaults`, `openwith`, `perm revoke`; `packages` file access and its limits | `AppPlatformTest.storeQueriesAndPackagesPermission` | `-JUnit AppPlatformTest -McVersion 26.1` |
| The App Store in the GUI: A serves the sample repository, B installs Net Check with Install and opens it; the desktop's dialog: Deny fails, relaunch from the launcher, Allow connects | `ecm_apps` (`AppScenarios.app_store`) | `-GameTests ecm_apps` |
| Double-clicking a `.py` in Files opens the Text Editor app, which can read only that file | `ecm_apps` (`AppScenarios.files_open_with`) | `-GameTests ecm_apps -GameTestOnly files_open_with` |
| Real prompts in the desktop's dialog (Allow, Deny, timeout, late remembered answer, console policy after `gui stop`); `gui launch` identity; `/dev/uictl launch` from an app stays in its group, `stop` refused; Open with -> the Editor app (only that file) and the chooser | `AppDesktopKernelTest` (real kernel, desktop) | `-JUnit AppDesktopKernelTest -McVersion 26.1` |
| `Notify` permission (refused, answered later), `Launch` / `Open` to the host with the sender's pid, `OpenFile` now or after `Welcome` | `UiServicesTest` | `-JUnit UiServicesTest -McVersion 26.1` |
| The store's Repositories page: add by URL (control: no address), name from the host, remove, at every size | `rust/crates/ecm-apps-ui/src/store/tests.rs` | `-Rust ecm-apps-ui` |
| A server packages and serves Chat; a cabled client installs it from the store, chats with the server's `chat`, gets a notification in the background | `ecm_apps` (`AppScenarios.chat_app`) | `-GameTests ecm_apps -GameTestOnly chat_app` |
| The Tech Village repository: ed25519 against `pkg`'s vector, the key from the seed, `/etc/pkg/repos`, the package server's signed files; generated computers and store phones pin it and have no Chat; `pkg serve` publishes only Chat, signed; a store phone installs Chat from the App Store over LTE and chats | `VillageRepoTest`, `sign::vector` (Rust); `ecm_cellular` `tech_village_repo`; `cell_tech_village` (tech client suite) | `-JUnit VillageRepoTest -McVersion 26.1`; `-Rust pkg`; `-GameTests ecm_cellular -GameTestOnly tech_village_repo`; `-ClientChecks -ClientSuite tech` |
| Settings -> Apps at compact / medium / wide; the launcher's installed apps | `rust/crates/ecm-apps/tests/apps.rs`; goldens `settings_apps*.png`, `desktop_apps.png` | `-Rust ecm-apps`; `-JUnit UiGoldenTest -McVersion 26.1` |
| The speaker mixer (formats, rates, a late stream joins after what was sent, ducking and its control, per-program audioctl, app strips, routes) and the Volume Mixer (`/dev/mixer` text and commands, persistence, permission) | `SpeakerAudioTest`, `VolumeMixerTest` | `-JUnit SpeakerAudioTest,VolumeMixerTest -McVersion 26.1` |
| Each app keeps its own logins: Discord signed in to an Auth server; Music, on the same realm, asks again (first contact for Music) and signs in with the same account; a changed realm key refused until Settings > Forget realm | `ecm_discord` (`AuthScenarios.auth_shared_realm`, `auth_pinned_key`) | `-GameTests ecm_discord -GameTestOnly auth_shared_realm,auth_pinned_key` |
| Large image resources (a 160x160 RGBA picture, ~100 KiB) uploaded before and while a window shows, on a page that redraws every 50 ms, through the app manager, and on the client-drawn path: every click still answered, the client picture equal to the compositor's frame (`apptest ui-image <n> [late|page]`) | `UiLargeResourceTest`; `ecm_apps` (`UiImageScenarios.gui_large_image`) | `-JUnit UiLargeResourceTest -McVersion 26.1`; `-GameTests ecm_apps -GameTestOnly gui_large_image` |
| Wake sockets in the kernel (`SO_ECM_WAKE` edges, control without it) | `rust/operating-system/rust/src/net/ipc_tests.rs` | `-Rust terminal-os` |
| The background platform on a phone: the first-time prompt (Don't allow / Allow), the pocket and a chest; ducking under a call; the background limit and its control; a wake socket in the pocket and its control | `ecm_phone` (`BackgroundScenarios`) | `-GameTests ecm_phone -GameTestOnly background_pocket,background_duck,background_limit,background_wake` |

`/ecm scenario spawn app_repo [auto|manual|fast]` builds the same two computers; `/ecm scenario commands app_repo` prints the script.

## Limits and open items

- **Prompts need the GUI.** Without a compositor and a connected shell (console mode, safe mode) undecided permissions are denied (or, by config, allowed) for the run. A prompt that timed out stays on screen until answered; its answer then only counts if remembered.
- **`/dev/uictl` and apps**: apps may only `launch` ([above](#integration-with-the-gui)); a confined app can still start installed apps, which run with their own permissions (as opening another app does on a phone).
- **Shell redirects don't reach programs** on this host (`process_spawn` ignores the fds), so the scenario writes the app's files from Python.
- **Duty cycling is wall-clock**, not CPU accounting (see above).
- **`location`** is enforced by a service that doesn't exist yet; it will call `check`. `notifications` is checked by the compositor (`Notify`).
- **Apps can open shell sessions** (`ipc_spawn_shell`) without a permission; everything they start there is confined like the app.
