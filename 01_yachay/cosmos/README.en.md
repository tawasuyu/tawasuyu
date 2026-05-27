# cosmos

> Astronomy with astronomical precision. Time · ephemerides · coordinates · images · astrology.

Rust suite for astronomical computation, validated against official ephemerides (JPL DE440/441, IAU 2006/2000A, IERS). Covers everything from time scales (UTC/TT/TAI/UT1) to WCS projections, through star catalogs, planetary positions, eclipses, transits, sundials, tides, tropical and sidereal astrology.

## Install

```sh
# CLI
cargo run --release -p cosmos-cli -- --help

# Llimphi app (sky map + interactive ephemerides)
cargo run --release -p cosmos-app-llimphi

# HTTP server
cargo run --release -p cosmos-server
```

## Compatibility

- **Linux / macOS / Windows** — all `core` crates compile without system deps.
- **Wawa** — cores compile to WASM (`cosmos-core`, `cosmos-time`, `cosmos-coords`, ...).
- **Web** — `cosmos-web` exposes a subset via WASM/JS.
- Validation against **JPL Horizons** and **AstroPy** in `cosmos-validation`.

## Crates

See the table in [README.md](README.md). Highlights: `cosmos-time`, `cosmos-coords`, `cosmos-ephemeris`, `cosmos-pointing`, `cosmos-catalog`, `cosmos-sky` (ergonomic facade), `cosmos-wcs`, `cosmos-astrology`, `cosmos-rise-set`, `cosmos-transits`, `cosmos-eclipses`, `cosmos-sundial`, `cosmos-tides`, `cosmos-leo`, plus `cosmos-cli`, `cosmos-server`, `cosmos-app-llimphi`, `cosmos-web`, `cosmos-validation`.

## Considerations

- **Zero client-side execution with user-sensitive data.** Latitude/longitude never leaves the binary without permission.
- DE files are downloaded **explicitly** via `cosmos-cli download`.
- Astrology is separable: if you don't want it, you don't link `cosmos-astrology`.
