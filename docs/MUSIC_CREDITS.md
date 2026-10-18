# Music credits

The songs the Tech Village music server (`musicd`, "Tech Village Radio") plays out of the box.
They ship in the mod jar under `src/main/resources/evanscomputermod/village/music/` and are
copied into the Data Center music computer's `/srv/music` when a village generates
(docs/TECH_VILLAGE_NETWORK.md, "Music").

| File | Title | Artist | Album | Genre | Length | Licence |
|---|---|---|---|---|---|---|
| `01-redstone-sunrise.mp3` | Redstone Sunrise | Tech Village Band | Tech Village Radio | Chiptune | 0:44 | CC0 1.0 |
| `02-village-fiber.mp3` | Village Fiber | Tech Village Band | Tech Village Radio | Ambient | 0:43 | CC0 1.0 |
| `03-data-center-nights.mp3` | Data Center Nights | Tech Village Band | Tech Village Radio | Synthwave | 0:40 | CC0 1.0 |

## Where they come from

All three are original works made for this mod by `scripts/make-village-music.py`: the notes,
the synthesis (sine, triangle, square, pulse and saw oscillators, noise drums, an echo), the
mix and the cover art (a sunset drawn pixel by pixel) are all code in that file. No samples,
recordings, loops or artwork from anywhere else were used, so there is no third-party
licence to check. "Tech Village Band" is a name for the generator, not a person.

The files are dedicated to the public domain under
[CC0 1.0 Universal](https://creativecommons.org/publicdomain/zero/1.0/): copy, change and
share them for any purpose without asking. Each MP3's ID3 tag says so in its copyright frame.

## Making them again

```
python scripts/make-village-music.py
```

It needs ffmpeg with libmp3lame on the PATH and writes the same audio every time
(32 kHz mono, 64 kb/s, ID3v2.3 with title, artist, album, album artist, year, genre, track
number and a 160 x 160 front cover).

## Adding more songs

Only add music whose licence lets the mod redistribute it (CC0, or CC BY with the credit in
this table). Record the source URL, the author, the licence and the date you checked it in a
new row, and keep files small (64-128 kb/s; `musicd` warns above 192 kb/s).
