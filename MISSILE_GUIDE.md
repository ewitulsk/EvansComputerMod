# Missile System Guide

## Overview

The missile system adds three tiers of ballistic missiles, a launcher block, and a Python computer module for programmatic control. Missiles can be fired manually via redstone or scripted entirely from a terminal using the `missile` Python module.

---

## Items & Blocks

### Missile Launcher

A metal block that holds and fires missiles. Place it like any block — it faces away from you. Found in the **Evans Computer Mod** creative tab alongside the missiles.

### Missile Types

| Tier | Item Name | Range | Speed | Turn Rate | Default Yield |
|------|-----------|-------|-------|-----------|---------------|
| MRBM | Medium Range Missile | 500 blocks | 2.5 b/tick | 4.0 deg/tick | 40% |
| LRBM | Long Range Missile | 2,000 blocks | 4.0 b/tick | 2.0 deg/tick | 60% |
| ICBM | ICBM | 8,000 blocks | 8.0 b/tick | 0.8 deg/tick | 90% |

**MRBM** is the most maneuverable — best for short-range guided strikes. **ICBM** is the fastest and most destructive but turns slowly, so aim carefully or set a GPS target.

Yield controls explosion power on a 0.0-1.0 scale, from 2 TNT (0.0) to 10 TNT (1.0).

---

## Step-by-Step: Launch a Missile from a Terminal

### 1. Get the items

Open your inventory, go to the **Evans Computer Mod** creative tab. Grab:
- **Terminal** (the computer)
- **Missile Launcher**

You don't need the missile items in your inventory — the `arm()` command loads missiles directly.

### 2. Place the blocks

Place the **Missile Launcher** somewhere with open sky above it. Note its coordinates (press F3). For this example, assume it's at **100, 64, 200**.

Place a **Terminal** wherever you like — it doesn't need to be next to the launcher. The Python API addresses the launcher by world coordinates, not proximity.

### 3. Open the terminal

Right-click the **Terminal** block. A screen with a Python prompt appears.

### 4. Configure the launcher

```python
import missile

# Point the API at your launcher (use its coordinates)
missile.set_launcher(100, 64, 200)   # returns True if found

# Aim: bearing is compass degrees, elevation is angle up
missile.set_bearing(90.0)             # 0=North, 90=East, 180=South, 270=West
missile.set_elevation(45.0)           # 5 to 85 degrees above horizontal
```

You'll see the launcher barrel physically rotate to match.

### 5. Set a GPS target (optional)

```python
missile.set_target(500, 64, -300)     # missile guides itself here after thrust
```

Skip this (or call `missile.clear_target()`) for an unguided ballistic arc.

### 6. Arm and launch

```python
missile.arm("lrbm")                   # "mrbm", "lrbm", or "icbm"
launch_id = missile.launch()          # returns a UUID string for tracking
```

The barrel turns green when loaded. After `launch()`, the missile spawns and flies.

### 7. Track the flight (optional)

```python
import json

while True:
    raw = missile.get_telemetry(launch_id)
    if not raw:
        break
    t = json.loads(raw)
    print(t['status'], round(t['x']), round(t['y']), round(t['z']))
    if t['status'] in ('impact', 'lost'):
        break

missile.clear_telemetry(launch_id)
```

Output looks like:

```
thrust 100 70 198
thrust 102 82 195
guiding 140 88 120
impact 500 64 -300
```

### 8. All-in-one script

```python
import missile, json

missile.set_launcher(100, 64, 200)
missile.set_bearing(90.0)
missile.set_elevation(45.0)
missile.set_target(500, 64, -300)
missile.arm("lrbm")
lid = missile.launch()

while True:
    raw = missile.get_telemetry(lid)
    if not raw:
        break
    t = json.loads(raw)
    print(t['status'], round(t['x']), round(t['y']), round(t['z']))
    if t['status'] in ('impact', 'lost'):
        break
missile.clear_telemetry(lid)
```

---

## Redstone Launch

You can also fire without a terminal:

1. Place a **Missile Launcher**
2. Use a terminal to configure it (bearing, elevation, arm, etc.)
3. Apply a **redstone signal** — the missile fires on the rising edge (unpowered to powered)

A button or lever flip triggers the launch. Useful for tripwire traps or button-activated defenses.

---

## Flight Phases

1. **Thrust** — Motor burns for the tier's duration. Accelerates to top speed. No gravity.
2. **Guiding** *(only with GPS target)* — Steers toward target at the tier's turn rate. Limited fuel.
3. **Coast** — No thrust, no steering. Gravity pulls it down. This is the final phase for unguided shots or after guidance fuel runs out.
4. **Impact** — Hits a block, detonates. 2-10 TNT based on yield.
5. **Lost** — Falls 64 blocks below world floor without hitting anything. Removed silently.

---

## Python API Reference

All functions require `missile.set_launcher(x, y, z)` first.

| Function | Returns | Description |
|----------|---------|-------------|
| `set_launcher(x, y, z)` | `bool` | Select a launcher by world coordinates |
| `set_bearing(deg)` | `bool` | Set compass bearing (0-360) |
| `set_elevation(deg)` | `bool` | Set pitch angle (5-85) |
| `get_bearing()` | `float` | Current bearing, or -1 |
| `get_elevation()` | `float` | Current elevation, or -1 |
| `set_target(x, y, z)` | `bool` | Set GPS guidance target |
| `clear_target()` | `bool` | Disable GPS guidance |
| `set_yield(scale)` | `bool` | Override warhead yield (0.0-1.0) |
| `arm(tier)` | `bool` | Load missile: "mrbm", "lrbm", "icbm" |
| `is_armed()` | `bool` | Check if missile loaded |
| `get_loaded_tier()` | `str` | Tier name or "" |
| `launch()` | `str` | Fire missile, returns telemetry UUID or "" |
| `get_telemetry(id)` | `str` | JSON snapshot: x,y,z,vx,vy,vz,fuel_pct,status,tta |
| `clear_telemetry(id)` | `None` | Free telemetry memory after flight ends |

## Telemetry JSON Fields

| Field | Type | Description |
|-------|------|-------------|
| `x`, `y`, `z` | float | Current position |
| `vx`, `vy`, `vz` | float | Velocity (blocks/tick) |
| `fuel_pct` | float | Guidance fuel remaining (0.0-1.0) |
| `status` | string | "thrust", "coast", "guiding", "impact", or "lost" |
| `tta` | float | Estimated seconds to impact (-1 if unknown) |

---

## Bearing Reference

```
        0 (North, -Z)
        |
270 ----+---- 90
(West)  |  (East)
        |
      180 (South, +Z)
```

## Tips

- **45 degrees** gives maximum range for unguided shots.
- **ICBM guidance is sluggish** (0.8 deg/tick). Aim roughly at the target; GPS fine-tunes.
- **MRBM is responsive** (4.0 deg/tick). Good for targets you can't aim at precisely.
- **Multiple launchers**: Call `set_launcher()` with different coords to switch. Each terminal tracks its own launcher independently.
- **No item cost**: `arm()` loads missiles in creative mode without consuming items.
