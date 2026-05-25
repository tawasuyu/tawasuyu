# cosmos-validation

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org/)

Internal validation harness and astronomical / astrological pipeline for
the [`eternal-*`](../) Rust workspace.

`cosmos-validation` is a dev-only crate (`publish = false`) that gates
every change to the surrounding ephemeris machinery against two
independent reference implementations:

- **JPL Horizons** (NASA/JPL ephemeris service, DE441)
- **Swiss Ephemeris** (Astrodienst, IAU 2006/2000A + DE441-derived `.se1`)

On top of the validation harness it ships the full apparent-position
pipeline and an astrological / observational layer:

| | |
|---|---|
| **SPK reading** | DE440 planet kernel + `sb441-n16.bsp` main-belt asteroids |
| **Analytic backends** | VSOP2013 (planets), ELP/MPP02 (Moon) |
| **Corrections** | Light-time, light deflection (Sun), stellar aberration, IAU 2006/2000A NPB |
| **Coordinate frames** | ICRF, GCRS, ecliptic of date, true equator and equinox of date (TET) |
| **Topocentric** | WGS-84 observer → ITRS → TET → alt / az |
| **Sidereal** | 8 ayanamshas (Lahiri, Fagan-Bradley, Krishnamurti, Raman, Yukteshwar, …) |
| **Houses** | Whole-Sign, Equal, Placidus, Koch, Regiomontanus, Campanus, Porphyry |
| **Lunar special points** | Mean + true (osculating) ascending node, mean + true Lilith |
| **Stars** | 26 named bright fixed stars (Hipparcos catalogue subset) |
| **Asteroids** | Ceres, Pallas, Juno, Vesta (Type 2 SPK) |
| **Events** | Rise / set / transit, lunar eclipses, global solar eclipses, local solar eclipses |

See [**PRECISION.md**](./PRECISION.md) for the full feature inventory,
the validation methodology, and the precision table per module.
See [**CHANGELOG.md**](./CHANGELOG.md) for the change history.

---

## Status

Early development. Public API will change. Each precision figure below
was validated against the version of Swiss Ephemeris 2.10.03 and the
DE440 kernel current at the time of last regeneration (2026-05-12).

| Pipeline | Match against Swiss Ephemeris |
|---|---|
| SPK geometry (planets wrt SSB) | < 1 µm position, < 1 nm/s velocity |
| Light-time corrected positions | sub-millimetre |
| Apparent (LT + S + LD + NPB) | sub-millarcsec on gas giants, 1–40 mas on inner planets |
| Tropical & sidereal longitudes | sub-arcsec at anchor; ±8″ across ±100 years (Lahiri) |
| Asc / MC | sub-arcsec across 4 reference charts |
| Whole-Sign / Placidus / Koch / Regiomontanus / Campanus / Porphyry cusps | sub-millarcsec (peak 0.001″) |
| Mean + true lunar node | sub-millarcsec (true), ±0.21″ (mean) |
| Mean + true Lilith | sub-arcsec |
| 26 fixed stars (apparent ecliptic of date) | < 0.04″ longitude, most sub-millarcsec |
| Main-belt asteroids (Ceres, Pallas, Juno, Vesta) | sub-arcsec |
| Topocentric Cartesian + alt/az | sub-arcsec |
| Rise / set / transit | ±100–200 s (horizon-convention difference) |
| Global solar eclipse times | **±0–4 s** (type-classification exact) |
| Lunar eclipse times | ±30–44 s (type exact) |
| Local solar eclipse times | sub-second to ±100 s (when fully visible) |
| ΔT (IERS table 1968–2030) | sub-second |

---

## Quick start

### Prerequisites

- Rust 1.70 or later (stable).
- ~120 MB free disk for the DE440 planet kernel.
- ~620 MB free disk for the `sb441-n16` asteroid kernel (optional).
- Python 3.8 or later with `pyswisseph` 2.10.x (only required for
  regenerating Swiss reference fixtures or running cross-validation).

### Install JPL kernels

```sh
mkdir -p ~/.local/share/ephemeris

# Required: planet kernel (114 MB)
curl -L https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/planets/de440.bsp \
     -o ~/.local/share/ephemeris/de440.bsp

# Optional: 16 main-belt asteroids including Ceres/Pallas/Juno/Vesta
curl -L https://ssd.jpl.nasa.gov/ftp/eph/small_bodies/asteroids_de441/sb441-n16.bsp \
     -o ~/.local/share/ephemeris/sb441-n16.bsp
```

### Build & run

```sh
# Build everything (release recommended)
cargo build --release -p cosmos-validation

# Run the gating regression tests
CELESTIAL_VALIDATION_SPK=~/.local/share/ephemeris/de440.bsp \
    cargo test --release -p cosmos-validation

# Inspect the precision of a specific module
./target/release/sidereal-check    --spk ~/.local/share/ephemeris/de440.bsp
./target/release/houses-check
./target/release/topocentric-check --spk ~/.local/share/ephemeris/de440.bsp
./target/release/altaz-check       --spk ~/.local/share/ephemeris/de440.bsp
./target/release/risetrans-check   --spk ~/.local/share/ephemeris/de440.bsp
./target/release/lunar-check       --spk ~/.local/share/ephemeris/de440.bsp
./target/release/stars-check       --spk ~/.local/share/ephemeris/de440.bsp
./target/release/asteroids-check   # uses both kernels by default
./target/release/eclipses-check    --spk ~/.local/share/ephemeris/de440.bsp
./target/release/local-eclipses-check --spk ~/.local/share/ephemeris/de440.bsp
```

### Use as a library

```rust
use eternal_validation::oracle::{Backend, Oracle};
use eternal_validation::fixture::{Corrections, Frame};

fn main() -> anyhow::Result<()> {
    let oracle = Oracle::new(Backend::Spk {
        kernel_path: "/path/to/de440.bsp".into(),
    })?;

    // Apparent Mars position as seen from Earth at TDB JD = 2460000.5,
    // in the true equator and equinox of date frame.
    let state = oracle.corrected_state(
        /* body   = */ 4,            // NAIF: Mars barycenter
        /* center = */ 399,          // NAIF: Earth body center
        /* jd_tdb = */ 2_460_000.5,
        Frame::TrueEquatorEquinoxOfDate,
        Corrections::APPARENT,       // LT + stellar aberration + light deflection
    )?;

    println!("Mars apparent (TET): pos_km = {:?}", state.pos_km);
    Ok(())
}
```

For a full tour of the available pipelines see the
[architecture overview](./PRECISION.md#architecture-overview) in
PRECISION.md.

---

## Repository layout

```
cosmos-validation/
├── Cargo.toml
├── README.md                      ← this file
├── PRECISION.md                   ← feature × precision inventory
├── CHANGELOG.md                   ← change history
├── src/
│   ├── lib.rs                     ← module re-exports
│   ├── oracle.rs                  ← Backend-agnostic state lookup
│   ├── fixture.rs                 ← JSON fixture schema
│   ├── horizons.rs                ← JPL Horizons API client (feature `fetch`)
│   ├── delta_t.rs                 ← IERS ΔT table 1968-2030
│   ├── sidereal.rs                ← Tropical + 8 ayanamshas + obliquity
│   ├── houses.rs                  ← 7 house systems
│   ├── lunar.rs                   ← Mean + true lunar node + Lilith
│   ├── fixed_stars.rs             ← 26 named bright stars + apparent pipeline
│   ├── asteroids.rs               ← Ceres / Pallas / Juno / Vesta
│   ├── topocentric.rs             ← WGS-84 observer + topocentric + alt/az
│   ├── rise_set.rs                ← Rise / set / transit finder
│   ├── eclipses.rs                ← Lunar, global-solar, local-solar
│   ├── report.rs                  ← Diff aggregation
│   └── bin/                       ← 11 inspection CLIs
├── scripts/                       ← 10 Python fixture generators (pyswisseph)
├── fixtures/                      ← Versioned JSON reference fixtures
│   ├── regression-de432/          ← gates with de432s.bsp
│   ├── regression-de440/          ← gates with de440.bsp (geometric)
│   ├── regression-de440-astrometric/
│   ├── regression-de440-apparent-vector/
│   ├── regression-de440-observer-astrometric/
│   ├── regression-de440-observer-apparent/
│   ├── regression-de440-swiss-apparent/
│   ├── regression-vsop2013/
│   ├── swiss-houses/
│   ├── swiss-altaz/
│   ├── swiss-risetrans/
│   ├── swiss-lunar/
│   ├── swiss-stars/
│   ├── swiss-asteroids/
│   ├── swiss-eclipses/
│   ├── swiss-local-eclipses/
│   └── swiss-topocentric/
└── tests/
    └── regression.rs              ← CI integration test
```

---

## Reproducing the fixtures

Every fixture under `fixtures/` is reproducible from the live Swiss /
Horizons sources. To refresh:

```sh
# Set up the Python reference environment (one-time)
python3 -m venv ~/.local/share/eternal-validation-venv
~/.local/share/eternal-validation-venv/bin/pip install pyswisseph

# Install Swiss data files (one-time)
mkdir -p ~/.local/share/swisseph
for f in sepl_18.se1 semo_18.se1 seas_18.se1 sefstars.txt; do
    curl -L https://github.com/aloistr/swisseph/raw/master/ephe/$f \
        -o ~/.local/share/swisseph/$f
done

# Regenerate Swiss reference fixtures
~/.local/share/eternal-validation-venv/bin/python3 \
    scripts/fetch_swiss.py            --ephe-path ~/.local/share/swisseph \
                                       --out fixtures/regression-de440-swiss-apparent/swiss.json
~/.local/share/eternal-validation-venv/bin/python3 scripts/fetch_swiss_houses.py    --out fixtures/swiss-houses/swiss-houses.json
~/.local/share/eternal-validation-venv/bin/python3 scripts/fetch_swiss_altaz.py     --out fixtures/swiss-altaz/swiss-altaz.json
~/.local/share/eternal-validation-venv/bin/python3 scripts/fetch_swiss_risetrans.py --out fixtures/swiss-risetrans/swiss-risetrans.json
~/.local/share/eternal-validation-venv/bin/python3 scripts/fetch_swiss_lunar.py     --out fixtures/swiss-lunar/swiss-lunar.json
~/.local/share/eternal-validation-venv/bin/python3 scripts/fetch_swiss_stars.py     --out fixtures/swiss-stars/swiss-stars.json
~/.local/share/eternal-validation-venv/bin/python3 scripts/fetch_swiss_asteroids.py --out fixtures/swiss-asteroids/swiss-asteroids.json
~/.local/share/eternal-validation-venv/bin/python3 scripts/fetch_swiss_eclipses.py  --out fixtures/swiss-eclipses/swiss-eclipses.json
~/.local/share/eternal-validation-venv/bin/python3 scripts/fetch_swiss_local_eclipses.py --out fixtures/swiss-local-eclipses/swiss-local-eclipses.json
~/.local/share/eternal-validation-venv/bin/python3 scripts/fetch_swiss_topocentric.py --out fixtures/swiss-topocentric/swiss-topocentric.json

# Regenerate Horizons fixtures (requires network access)
cargo run --release -p cosmos-validation --features fetch --bin precision-report -- \
    fetch --backend spk
cargo run --release -p cosmos-validation --features fetch --bin precision-report -- \
    fetch --backend spk-astrometric
cargo run --release -p cosmos-validation --features fetch --bin precision-report -- \
    fetch --backend spk-apparent-vector
cargo run --release -p cosmos-validation --features fetch --bin precision-report -- \
    fetch --backend spk-observer-astrometric
cargo run --release -p cosmos-validation --features fetch --bin precision-report -- \
    fetch --backend spk-observer-apparent
cargo run --release -p cosmos-validation --features fetch --bin precision-report -- \
    fetch --backend vsop
```

---

## Cargo features

| Feature | Effect |
|---|---|
| `default` | None — minimal build for using the library only |
| `fetch` | Enables the `horizons` module (pulls in `reqwest` + `tokio`) |
| `serde` *(workspace)* | Inherited through `cosmos-time`, `cosmos-core`, `cosmos-coords` |

Add to your `Cargo.toml`:

```toml
[dev-dependencies]
cosmos-validation = { path = "../cosmos-validation", features = ["fetch"] }
```

(This crate is not published to crates.io. The version in
`Cargo.toml` tracks the workspace.)

---

## Contributing

Pull requests welcome. Please read **CONTRIBUTING.md** (forthcoming)
before opening a PR.

For now, the rough conventions:

- **Validation first.** Every new feature should ship with a Swiss-Ephemeris
  or Horizons reference fixture and an inspection CLI that prints a diff
  table. The pattern is in any of the `*_check.rs` files under `src/bin/`.
- **Precision-budget pull requests.** When you tighten or loosen a residual
  against Swiss/Horizons, update both [PRECISION.md](./PRECISION.md) and
  the relevant `regression-*` fixture tolerances.
- **No silent fall-backs.** If the local code can't reproduce a Swiss /
  Horizons answer, document the gap and its cause rather than relaxing the
  comparison without explanation.
- **Open issues for unfinished work.** Use the
  [suggestions list](./PRECISION.md#suggested-next-work) as a starting
  point.

---

## License

`cosmos-validation` is licensed under the **Apache License, Version 2.0**
([LICENSE-APACHE](../LICENSE-APACHE) or
<https://www.apache.org/licenses/LICENSE-2.0>),
matching the rest of the `eternal-*` workspace. See [NOTICE](../NOTICE)
for upstream attribution.

This crate ports algorithms from [Swiss Ephemeris](https://www.astro.com/swisseph/)
(Astrodienst AG, AGPL-3 / commercial dual-licensed) — specifically the
house-system implementations in `src/houses.rs`. Those ports are credited
in-source. The ports re-implement Swiss algorithms in original Rust code
and do not redistribute Swiss data files; users who want the binary
`.se1` reference files for cross-validation must download them
separately from Swiss's [public mirror](https://github.com/aloistr/swisseph/tree/master/ephe).

---

## Acknowledgements

- **Greg Aker** — original author of [celestial](https://github.com/gaker/celestial),
  the upstream project this fork derives from. The core astronomy crates
  (`cosmos-core`, `cosmos-time`, `cosmos-coords`, `cosmos-ephemeris`,
  `cosmos-images`, `cosmos-pointing`, `cosmos-wcs`, `cosmos-catalog`)
  are his work; `cosmos-validation` is the new layer added in this fork
  by Sergio Velásquez Zeballos with Claude (Anthropic). The
  `cosmos-sky` façade and the `cosmos-astrology` symbolic layer were
  added subsequently in the same collaboration.
- **JPL Horizons** and the **NAIF SPICE Toolkit** — for the DE441 / DE440
  ephemerides and the SPK format that underpins every precision claim
  in this document.
- **Astrodienst (Swiss Ephemeris team)** — for the AGPL Swiss Ephemeris
  reference implementation, the `swehouse.c` house algorithms, the
  `sefstars.txt` fixed-star catalogue, and Dieter Koch's continuous work
  on the IAU reductions for astrological precision.
- **IERS Bulletin A / EOP C04** — for the observed ΔT and EOP values
  bundled in `celestial-eop-data`.
- **IAU SOFA** — for the precession / nutation / frame transformations
  re-implemented in `cosmos-core`.

### With thanks to

For their guidance, conversations, and inspiration that shaped the
direction of the astrology pipeline built on top of this validation
harness:

- **Roberto Reiley**
- **Germán Rosas**
- **Juan Velásquez**
- **Guillermo Velásquez**
