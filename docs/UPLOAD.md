# File upload

Players copy a file from their real computer (Windows, Linux or macOS) into a
Terminal or ePhone in the game. It is an optional add-on,
`evanscomputermod-upload-mc<version>.jar`, built by `copy-jar.sh` next to the
main mod.

## Install

Put the add-on in the server's mods folder **and** in each player's mods folder
(single player: just yours). Either side may lack it and still connect; upload
is then simply not offered. The server finds out per player (it checks the
player's game for the add-on's network channel), so on a server where some
players have it and some don't, only those who do see the Upload button.

## Use

- **Console:** `upload [DIR]` (default `/home`). Your system's file picker opens
  in your game; the file lands in `DIR` under its own name. A taken name gets
  `" (1)"`, `" (2)"`, ... Ctrl+T stops waiting. `upload status` says whether you
  can upload to this computer and, if not, why.
- **Files:** the **Upload** button (on the ePhone, in the `...` menu) uploads into
  the folder shown. It appears only while the player using the computer can
  upload. The ePhone now has Files on its home grid (it sees the documents
  folder, `/home`).
- **Python and Lua:** `shell.upload_supported()` returns `"ready"`,
  `"not_installed"`, `"no_client"` or `"not_permitted"`. `shell.upload([dir])`
  returns the new file's path; Python raises `OSError` with the reason, Lua
  returns `nil, reason`.
- **Programs:** `ecm_host_abi::upload` (abi/child-abi.toml `[imports.upload]`).

"The player using the computer" is whoever last typed or clicked on it: the
ePhone's holder, the player at the Terminal. They must still be able to use it
when the upload starts (in range, or with its screen open).

## Server settings

`config/evanscomputermod_upload-server.toml` (in the world's `serverconfig`):

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Uploads on or off |
| `operatorsOnly` | `false` | Only operators (permission level 2) may upload |
| `maxFileSizeKiB` | `16384` | The largest file (16 MiB) |
| `maxComputerStorageMiB` | `256` | Refuse an upload that would take a computer's files over this (0 = no limit) |

Computers have no other storage quota; this limit applies to uploads only.

## How it works

- **Main mod (hardware):** `computer/upload/UploadPort`, one per computer, is the
  upload port behind the `upload_*` host functions. `ComputerTargets` notes the
  player behind each keyboard or pointer packet. A file is written as a hidden
  `.upload-<ticket>.part` beside its destination and renamed into place when
  every byte is in. One upload per computer at a time; it belongs to the
  program that began it and is cancelled when that program exits.
- **SPI:** `api/upload/UploadBackend` (installed by the add-on through
  `UploadBackends.install`), `UploadSink` (one upload, as the backend sees it),
  `UploadStatus` (states and errors).
- **Add-on (`evanscomputermod-upload`):** `ServerUploads` is the backend; the
  payloads (`UploadPackets`: Pick, Begin, Chunk, End, Cancel, Result) are
  registered as optional. `ClientUploads` opens the picker with LWJGL's
  tinyfiledialogs (which Minecraft already ships for every OS) on its own thread,
  reads the file and sends it in 30,000-byte chunks, 8 per client tick (about
  4.8 MB/s). A pick times out after 10 minutes, a transfer after 30 s without a
  chunk, and a player who leaves loses their upload. Packets for an upload are
  taken only from the player it was sent to.

## Tests

See TESTING.md ("File upload"). The scenarios `upload_terminal` and
`upload_files_app` (`-Upload -GameTests ecm_upload`) arm the player's pick
(docs/SCENARIO_EXCEPTIONS.md, "upload file picker"); the real game's path, the
picker's stand-in aside, is the hidden-client check
`-Upload -ClientChecks -ClientSuite upload`.
