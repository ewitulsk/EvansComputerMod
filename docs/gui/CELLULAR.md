# Cellular v1 (superseded)

**Superseded by [CELLULAR_V2.md](CELLULAR_V2.md).** Cellular v1 (ROADMAP P7) never shipped and was
deleted in Cellular v2 phase 4: the Cell Tower block (a base station that ran attach, handover and the
carrier's subscriber table inside Java), the carrier screen, `/ecm cell`, the v1 modem engine and the
kernel's `wwan0` NIC with its `cell_*` host functions. Worlds never had them, so there is no migration.

What v1 built and v2 kept, and where it lives now:

| v1 | Now |
| --- | --- |
| LTE Sector Antenna, mmWave Array Antenna (`lte_antenna`, `mmwave_antenna`) | The same passive panels, fed by an SDR's coax ([CELLULAR_V2.md](CELLULAR_V2.md), `CellAntennas`, `ExciterLink.Panel`) |
| LTE / mmWave Modem modules (`lte_modem`, `mmwave_modem`) | Framed radios driven by `celld` (`FramedModemModule`, peripheral `framed_radio`) |
| SIM cards (`sim_card`) | Smart cards written by a Card Reader/Writer (`cored sim new`, `simtool`) |
| Channel plans, powers, CQI table (`CellTech`, `CellPhy`) | `radio/framed/PhyProfile`, `CqiTable` ([SDR_PROGRAMS.md](../radio/SDR_PROGRAMS.md#framed-phy-mode-advanced-sdr)) |
| Tower attach, handover, carriers, quotas | Programs: `celld-site` (cell site), `cored` (carrier core), `celld` (phone) on the `ecm-cell` protocol |
| Scenarios `cell_lte_far`, `cell_mmwave_hill`, `cell_handover`, `cell_wrong_carrier` | Rebuilt on player-built networks; `cell_handover` is now `cell_roaming` ([CELLULAR_V2.md, "As built: phase 4"](CELLULAR_V2.md#as-built-phase-4)) |
