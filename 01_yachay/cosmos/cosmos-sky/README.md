# cosmos-sky

Ergonomic public façade over the `eternal-*` astronomy crates.

[![License: Apache 2.0](https://img.shields.io/crates/l/cosmos-sky)](https://gitea.tawasuyu.net/sergio/eternal)

Hides the orchestration of time scales, ephemeris kernels, IAU rotations, and topocentric reductions behind three high-level types: `Instant`, `Observer`, and `EphemerisSession`. Every number forwards to the same validated routines that gate the regression harness of the lower layers — precision is identical; the only thing added is ergonomics.

## Installation

```toml
[dependencies]
cosmos-sky = "0.1"
```

## Quick start

```rust
use eternal_sky::{Body, EphemerisSession, Instant, Observer, SessionConfig};

let session = EphemerisSession::open(SessionConfig::vsop2013())?;
let observer = Observer::from_degrees(10.4806, -66.9036, 900.0);
let when = Instant::from_civil_local(1987, 3, 14, 5, 22, 0.0, -240)?;

let mars = session.body_apparent(Body::Mars, when, Some(&observer))?;

println!("Mars λ = {:.4}°  β = {:.4}°  alt = {:.2}°",
    mars.ecliptic_of_date.longitude_deg(),
    mars.ecliptic_of_date.latitude_deg(),
    mars.topocentric_horizon.unwrap().altitude_deg(),
);
# Ok::<_, eternal_sky::SkyError>(())
```

## Modules

| Module          | Purpose                                                         |
|-----------------|-----------------------------------------------------------------|
| `instant`       | Civil time (UTC) with on-demand TT/TDB/UT1/JD-TDB conversion    |
| `observer`      | Geodetic location on the WGS-84 ellipsoid                       |
| `body`          | 22-variant enum for Sun/Moon/planets/nodes/Lilith/asteroids     |
| `session`       | `EphemerisSession`: opened SPK or analytical backend            |
| `apparent`      | `ApparentPosition` bundles ecliptic + equatorial + horizon      |
| `event_search`  | `find_root` / `find_all_roots` bisector over time               |
| `delta_t`       | IERS ΔT table lookup                                            |

## Bodies

| Variant | Backend | Notes |
|---|---|---|
| Sun, Moon | SPK or VSOP/ELP | Full apparent pipeline on SPK |
| Mercury–Pluto | SPK or VSOP | Pluto is a planetary barycenter for symmetry |
| MeanNode, MeanLilith | analytical | Pure series, no SPK needed |
| TrueNode, TrueLilith | SPK | Osculating from the Moon's instantaneous state |
| Ceres, Pallas, Juno, Vesta | SPK + asteroid kernel | Use `with_asteroid_kernel(path)` |
| Chiron, Pholus, Eris, Sedna | SPK Type 21 (parsing wired, interpolation TBD) | Segment metadata is read; the Newhall MDA interpolation step is the open work item |

## Backends

```rust
// Analytical (no kernel files, ~arcsec precision):
SessionConfig::vsop2013()

// JPL DE-series (sub-mas precision, full apparent pipeline):
SessionConfig::with_spk("/path/to/de440.bsp")

// With asteroids:
SessionConfig::with_spk("/path/to/de440.bsp")
    .with_asteroid_kernel("/path/to/sb441-n16.bsp")
```

## Event search

`find_root` is a generic bisector over instants. Three presets cover the common cadences:

```rust
use eternal_sky::{find_root, SearchOptions};

let next_zero = find_root(
    t0, t1,
    |t| Ok(some_quantity(t)?),
    SearchOptions::DAILY,        // PRECISE, HOURLY, DAILY
)?;
```

Every astrology-layer forecast (returns, transits, station-finding) builds on this primitive.

## Precision

| Path                  | Source                | Typical precision |
|-----------------------|-----------------------|--------------------|
| Major planets (SPK)   | DE440 / DE441 Chebyshev | sub-meter position |
| Major planets (VSOP)  | VSOP2013 ablated series | < 5,000 km (inner) |
| Moon (SPK)            | DE Chebyshev          | sub-meter position |
| Moon (analytical)     | ELP/MPP02             | < 5 km vs DE441    |
| Asteroids (SPK)       | sb441-n16             | sub-meter          |
| Time scales           | IERS leap seconds + ΔT table 1968–2030 | sub-second UT1, sub-µs TT/TDB |
| Coordinate transforms | IAU 2006/2000A NPB    | sub-mas            |

## Design

- **No parallel implementations.** Every internal call eventually hits `cosmos-validation::oracle::Oracle`, which is the same engine the regression harness uses to gate the rest of the workspace.
- **Astronomy first.** This crate makes no astrological claim. The astrology layer lives in `cosmos-astrology`.
- **Cheap to clone.** `Instant`, `Observer`, `Body` are all `Copy`. `EphemerisSession` owns its kernel handles and is cheap to share by reference.

## License

Licensed under the Apache License, Version 2.0
([LICENSE-APACHE](../LICENSE-APACHE) or
<https://www.apache.org/licenses/LICENSE-2.0>).

## Acknowledgements

This crate is part of the [eternal](../) workspace — a fork of
[celestial](https://github.com/gaker/celestial) by Greg Aker — and
was added by Sergio Velásquez Zeballos in collaboration with
Claude (Anthropic) to provide an ergonomic API surface for the
validated astronomy living in `cosmos-validation`.

### With thanks to

For their guidance, conversations, and inspiration that shaped the
direction of the astronomy façade and the astrology pipeline built
on top of it:

- **Roberto Reiley**
- **Germán Rosas**
- **Juan Velásquez**
- **Guillermo Velásquez**
