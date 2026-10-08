# Radio medium benchmarks

`MediumBench` (`src/test/java/.../radio/medium/MediumBench.java`, lane 3F/8C) drives the real
`WorldRadioMedium` with 100, 1,000 and 10,000 radios on 2.4 GHz channels 1/6/11, on one thread and
on four, in two layouts:

- **dense**: everyone within link-budget range (radios 8 blocks apart on average, 15 dBm), so every
  frame is checked against every co-channel radio;
- **sparse**: 0 dBm radios spread over 100 km, so almost nobody hears anybody.

The world is a flat `GridWorld` (dirt at y 64). Each run first ticks the medium three times, which
spends the per-tick ray budget tracing links for the new radios (the only place rays are cast), then
transmits 100-byte OFDM-24 frames from random radios. It asserts that `transmit` never reads the world
(no raycasts on the hot path) and only sane minimums (dense > 30 frames/s, sparse > 1,000 frames/s):
the numbers are measurements, not limits.

Columns:

- **frames/s**: `transmit` calls per second (delivery to every receiver included);
- **checks/s**: receivers considered per second;
- **B/frame**: bytes allocated on the transmitting threads per frame;
- **tick**: wall time of one medium tick while the link cache is filling, and the links it traced.

Run it with:

```powershell
scripts\Test.ps1 -Area radio-bench -JUnit MediumBench -McVersion 26.1 -NoStage
```

## 2026-10-08 (branch `radio/aero-release`)

AMD Ryzen 7 9800X3D (8 cores / 16 threads), Windows 11, Gradle test JVM. Receipt:
`artifacts/radio-sta-bench-20261008-033055` (JUnit `MediumBench`, 1 test, pass; an earlier run,
`artifacts/radio-bench-20261008-032508`, and a third, `artifacts/radio-gate-junit-20261008-035816`, gave the same picture within ~15%).

| Radios | Layout | Threads | frames/s | checks/s | B/frame | tick (ms) | links traced/tick |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | dense | 1 | 54,629 | 1,766,805 | 1,732 | 3.65 | 762 |
| 100 | dense | 4 | 159,924 | 5,172,753 | 1,933 | 4.08 | 763 |
| 100 | sparse | 1 | 3,342,637 | 64,764 | 153 | 0.12 | 0 |
| 100 | sparse | 4 | 10,879,617 | 219,904 | 157 | 0.10 | 0 |
| 1,000 | dense | 1 | 5,290 | 1,758,108 | 16,104 | 6.74 | 383 |
| 1,000 | dense | 4 | 18,345 | 6,096,559 | 16,104 | 6.00 | 376 |
| 1,000 | sparse | 1 | 675,824 | 247,014 | 170 | 0.73 | 1 |
| 1,000 | sparse | 4 | 1,980,786 | 717,210 | 169 | 0.69 | 1 |
| 10,000 | dense | 1 | 539 | 1,797,747 | 160,106 | 57.64 | 341 |
| 10,000 | dense | 4 | 1,814 | 6,043,616 | 160,637 | 55.36 | 348 |
| 10,000 | sparse | 1 | 27,945 | 94,011 | 314 | 8.07 | 89 |
| 10,000 | sparse | 4 | 133,502 | 450,234 | 314 | 6.52 | 89 |

(The first 100-radio dense row of each run is a warm-up and is left out.)

### What the numbers say

- **Delivery scales with cores.** Four threads give 3.0-3.5x the frames/s of one at every size;
  receiver checks run at about 1.8 M/s per thread regardless of radio count, so a dense frame costs
  in proportion to the radios that can hear it.
- **Sparse worlds are cheap.** With nobody in range, `transmit` is a band-index lookup: 28k-3.3M
  frames/s on one thread from 10,000 down to 100 radios.
- **No raycasts on the hot path.** The world-read counter did not move during any transmit phase.
- **Gap: the hot path allocates.** The spec asks for no per-frame allocation. Dense frames allocate
  about 48 bytes per co-channel radio (1.7 KB at 100, 16 KB at 1,000, 160 KB at 10,000 radios, a
  third of them on the frame's channel), so the per-receiver delivery path still creates objects. Sparse frames
  allocate ~150-300 B. Not fixed in this lane; tracked here.
- **Watch: the registration burst at 10,000 dense radios.** The first medium ticks after 10,000 radios
  appear at once took ~56 ms each (more than a 50 ms server tick) while tracing only ~345 links: the
  time is pair discovery (every new radio queues up to 64 neighbours within 128 blocks, so ~640,000
  queue operations), not ray tracing, and it runs on the server thread. The ray budget itself held
  (links traced per tick stayed bounded). It is a one-off burst when a crowd of radios loads, not the
  steady state, but spreading discovery over ticks or workers is the next scaling step.
