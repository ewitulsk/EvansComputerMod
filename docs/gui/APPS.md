# Apps — EvansComputerMod

The app platform core (roadmap [P4](ROADMAP.md#p4--app-platform)): packages, the `pkg` package manager and repositories, the app manager, permissions, saved state and the ePhone background rules. The GUI parts (App Store app, permission dialog, file picker, chooser) come later and plug into the interfaces in [For the compositor and the phone](#for-the-compositor-and-the-phone).

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

`publisher_key` and `signature` are optional. `file` must be a plain file name. Clients take the newest version per id.

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
pkg open <path>                           open a file with its default app
pkg build <dir> [out] [--sign <key>]      pkg keygen [file]
pkg serve <dir> [port]                    default port 8080
```

Installing unpacks into `/apps/<id>/` (replacing an older version), creates `/var/apps/<id>/` and tells the app manager to reindex through [`app_ctl`](#app_ctl) (a host call, not a file watch: it is immediate and needs no polling). `pkg` itself is a normal program typed in the terminal, so it is unconfined.

## Identity

- **An app is a process group.** `AppManager.launch(id, args)` registers the group, then spawns the entry into it (`main.wasm` directly; `main.py` as `python /apps/<id>/main.py`), so the very first instruction already runs as the app. Environment: `COLUMNS`, `LINES`, `ECM_APP_ID`, `ECM_APP_DATA`.
- **Children inherit it.** Child programs are started by the kernel. When the kernel spawns a program while it is handling a request from process P (P typed a line into its shell session, as sshd does), `ComputerInstance.process_spawn` puts the new program into P's app group (`NetIpcBridge.dispatchingPid()` → `AppManager.groupForSpawn`). No ABI change was needed: the kernel only spawns from inside such a request or from its own console.
- **Terminal commands are not apps.** Programs typed in the terminal (or started by unconfined programs) are in no app group and are unconfined, exactly as before.
- `appIdOf(pid)` / `instanceOf(pid)` work for every member; an instance lives until all its members have exited.

## Permissions

Names from [UI Protocol §10](UI_PROTOCOL.md#10--apps-and-permissions), plus `packages` (install, launch and manage apps through `app_ctl`; for the App Store):

`network`, `radio`, `cellular`, `controller`, `location`, `notifications`, `files:documents`, `files:all`, `packages`, `peripherals:<type>`, `background:refresh|audio|radio|transfer|navigation`.

How a check is decided:

1. Not declared in `app.toml` → denied, never asked.
2. A remembered answer (`/var/lib/apps/db.json`) → that answer.
3. An answer for this run only (`remember = false`) → that answer.
4. `background:*` → denied (approved by the player in Settings / the phone shell, never mid-run).
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
| `periph_call`, `periph_methods` | by type: `sdr amplifier antenna dish microwave_radio wifi` → `radio`; `lte_modem mmwave_modem` → `cellular`; `xbox_controller controller_receiver` → `controller`; else `peripherals:<type>` | error frame "permission denied" |
| `periph_wait_event` | only events of peripherals already allowed (filtered when posted) | — |
| `redstone_*` | `peripherals:redstone` | -13 / 0 |
| `/dev/sdr*` (`path_open`) | `radio` | `ACCES` |
| `/dev/audio*` | `peripherals:speaker` | `ACCES` |
| `path_open`, `path_filestat_get`, `path_create_directory`, `path_unlink_file`, `path_remove_directory`, `file_*`, `video_open` | the file rules above | `NOENT`/`ACCES`, -13 |
| `app_ctl` (anything but `whoami`) | `packages` | -13 |

Every socket path starts with `sock_socket`, so the other `sock_*` calls need no check. `location` and `notifications` belong to services that don't exist yet; their owners call `AppManager.check(pid, perm)`. `path_remove_directory` (empty directories) was implemented on the way, since `pkg remove` needs it.

## Saved state

`WillSuspend` → `SaveState` → `Restore` (UI Protocol §11), stored in `/var/apps/<id>/state` (written atomically, at most 256 KiB).

- **Terminals at shutdown and reboot (PC policy).** `TerminalBlockEntity.shutdownComputer` calls `ComputerInstance.shutdownApps()` before interrupting the computer (`close()` does it too, idempotently). `AppManager.shutdown()` records the running apps (`/var/lib/apps/session`), sends `WillSuspend{deadline}` to each through `AppEvents.willSuspend`, waits until every app that got it has answered or the deadline (`apps.saveStateDeadlineMs`, 500) passes — one deadline for all — and stops the apps. An app that misses the deadline keeps its previous state.
- **Next launch.** When the app's connection says `Hello`, the compositor sets `restore_follows = peekRestore(id)` and sends `takeRestore(id)` as `Restore` right after `Welcome`. The file stays until the app saves again, so a crash right after a restore loses nothing. `lastSession()` lists what was running (a desktop may relaunch it).
- **Without a compositor** nothing has a UI connection, `willSuspend` returns false, and shutdown doesn't wait.

## Background (ePhone)

`AppManager.setPolicy(LifecyclePolicy.PHONE)`; `BackgroundScheduler` (`apps.background()`) on the pause API (an app = a process group; suspended = group paused).

- **Suspend / resume.** `suspend(app)` collects its state with the deadline, then pauses the group. `resume(app)` resumes it (and ends a refresh window it was in).
- **Refresh wakes.** For a suspended app that asks for `refresh` and holds `background:refresh`: every `apps.refreshIntervalMinutes` (5) the group is resumed for at most `apps.refreshBudgetMs` (2000) of wall time, then paused again. If the computer sleeps, it is woken for the window with `ComputerInstance.wakeFor` (installed as the scheduler's `SleepControl`). `BackgroundDone` (`onBackgroundDone(pid)`) ends the window early. One window at a time; due apps wait for the next `tick`. Drive it with `tick(nowMs)` from the phone's server tick, or `start()` for an own 1 s timer. `refreshNow(app)` starts a window at once and returns it (`isOver`, `endedBy` = `budget` / `done` / `resumed`, `ranMs`).
- **Exception modes.** `startMode(app, mode)` when the app is really doing it (audio playing, a transfer running); it needs the mode in `app.toml` and `background:<mode>` allowed. While active, a suspended app is duty-cycled: resumed for `100 ms × apps.backgroundCpuPercent / 100` (10 % → 10 ms) of every 100 ms. There is no per-process CPU accounting, so the cap is wall-clock run time per group; a group with several busy threads can use that share on each. `stopMode` pauses it again. `activeModes()` (app id → modes) feeds the status bar; `AppEvents.backgroundModesChanged` reports changes.
- **Wasmtime limit** ([UI Protocol §15](UI_PROTOCOL.md#pausing-what-works)): a group in a pure compute loop only stops at its next host call there, so budgets and duty cycles are approximate for such apps; `ProcessManager.awaitHalted` tells whether a pause has landed.

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

## app_ctl

The child import behind `pkg` (`abi/child-abi.toml` `[imports.apps]`, Rust `ecm_host_abi::apps`): `app_ctl(req, len, reply, cap, status_out) -> full reply length`. Requests are words separated by `\n`:

`whoami` · `reindex` · `list` · `ps` · `launch <id> [args…]` (→ pid) · `status <pid>` (`running` / `suspended` / `exited <code>`, then `denied <perm>` lines) · `output <pid>` (buffered output of the app's main process, up to `cap` bytes, taken once) · `kill <pid>` · `open <path>` · `default <.ext> <id|->` · `session` · `perm list [id]` · `perm grant|deny|reset <id> <permission>`.

Status: 0 ok, -2 not found, -13 not permitted, -22 bad request, -35 choose an app. Confined processes may only use `whoami` unless they hold `packages`. The simulator has no app manager (-1).

## Tests

| What | Where | Run |
| --- | --- | --- |
| TOML subset, manifest, archives, index, trust, HTTP, repo list (Rust) | `rust/wasm-programs/pkg/src/*` | `-Rust pkg` |
| Java manifest on the shared vectors | `AppManifestVectorsTest` | `-JUnit AppManifestVectorsTest -McVersion 26.1` |
| Deny prompt and remembered answer; undeclared and console-mode denials; child inherits the app's permissions; open-with grant allows only that file (plus own data, own code read-only, others' data and the app DB never); state survives a restart; refresh budget, BackgroundDone, no-permission control, exception mode | `AppPlatformTest` (real kernel, `apptest` probe) | `-JUnit AppPlatformTest -McVersion 26.1` |
| Computer A serves a repository, B installs over a cable with `pkg`, network denied until granted | `ecm_apps` (`AppTests`, `AppScenarios.app_repo`) | `-GameTests ecm_apps` |

`/ecm scenario spawn app_repo [auto|manual|fast]` builds the same two computers; `/ecm scenario commands app_repo` prints the script.

## Limits and open items

- **No GUI yet.** The App Store app, the permission dialog, the file picker and the chooser come with the compositor and desktop lanes; until then console mode denies (or, by config, allows) undecided permissions.
- **Shell redirects don't reach programs** on this host (`process_spawn` ignores the fds), so the scenario writes the app's files from Python.
- **Duty cycling is wall-clock**, not CPU accounting (see above).
- **`location` and `notifications`** are enforced by services that don't exist yet; they call `check`.
- **Apps can open shell sessions** (`ipc_spawn_shell`) without a permission; everything they start there is confined like the app.
