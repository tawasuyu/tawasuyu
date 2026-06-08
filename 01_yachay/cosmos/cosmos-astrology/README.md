# cosmos-astrology

The astrology-specific layer of the `eternal` workspace, built on the [`cosmos-sky`](../cosmos-sky/) façade.

[![License: Apache 2.0](https://img.shields.io/crates/l/cosmos-astrology)](https://gitea.tawasuyu.net/sergio/eternal)

A typed pipeline that turns *(when, where)* into a `NatalChart`: four angles, twelve house cusps in the chosen system, every requested body placed in its sign and house with retrograde flag — plus a full forecasting toolkit: aspects, returns, progressions, solar arc, the classical primary-direction trilogy (Placidus, Regiomontanus, Campanus), transits, stations, synastry, midpoint composites, Arabic Parts, Hellenistic profections, lunar phases, and eclipses-on-natal.

## Disclaimer

Astrology is a symbolic system with deep cultural and personal significance for many people. This crate computes its traditional constructs faithfully but **takes no position** on whether those constructs describe, predict, or explain anything about an individual's life. Treat the output as a *language*, not as data. The precision claims in this README refer strictly to the astronomical inputs (planet positions, time scales, IAU rotations); they say nothing about the validity of the astrological interpretations the user may build on top.

## Installation

```toml
[dependencies]
cosmos-astrology = "0.1"
cosmos-sky = "0.1"
```

## Feature matrix

| Concept | API | Tests |
|---|---|---|
| Natal chart (7 house systems, tropical or sidereal) | `NatalChart::compute` | ✅ |
| Whole-Sign, Equal, Porphyry, Placidus, Koch, Regiomontanus, Campanus | `HouseSystem` | ✅ |
| 8 ayanamshas (Lahiri, Fagan-Bradley, Krishnamurti, Raman, …) | `Zodiac::Sidereal(Ayanamsha::*)` | ✅ |
| 22 bodies — luminaries, planets, nodes m+v, Lilith m+v, asteroids | `BodySet` | ✅ |
| Mundane helpers (DA, semi-arcs, Placidus quadrant `m`) | `mundane::*` | ✅ |
| Aspects (12 kinds, applying/separating, orb table) | `find_aspects` | ✅ |
| Planetary returns (Sun / Moon / any body) | `next_return` | ✅ |
| Progressions: Secondary, Tertiary, Minor | `secondary_progression`, … | ✅ |
| Solar Arc directions (TrueProgressedSun, Naibod) | `solar_arc_true`, `solar_arc_naibod` | ✅ |
| Primary directions — Placidus mundane, **Regiomontanus**, **Campanus** | `direct`, `direct_to_aspect`, `all_directions_with_aspects` | ✅ |
| Direction keys (Ptolemy 1°/yr, Naibod 0°59'08"/yr) | `DirectionKey` | ✅ |
| Transits — current snapshot + next exact root-finder | `find_current_transits`, `find_next_exact_transit` | ✅ |
| Planetary stations (retrograde / direct) | `next_station`, `all_stations` | ✅ |
| Synastry — cross-aspects between two charts | `find_synastry_aspects` | ✅ |
| Composite — midpoint chart | `composite` | ✅ |
| Arabic Parts (7 canonical Lots + custom) | `compute_lot`, `all_lots`, `custom_lot` | ✅ |
| Hellenistic profections (annual + monthly + Lord of the Year) | `annual_profection`, `monthly_profection`, `profection_at` | ✅ |
| Lunar phases (4 canonical + 8-fold lunation classification) | `next_lunar_phase`, `next_canonical_phase`, `classify_lunation_phase` | ✅ |
| Eclipses (solar / lunar) on natal points | `eclipses_on_natal`, `next_solar_eclipse`, `next_lunar_eclipse` | ✅ |
| Generic event root-finder over time | `eternal_sky::find_root` | ✅ |

102 tests across `cosmos-sky` + `cosmos-astrology` gate the precision and behaviour of these features against direct calls into the validated underlying machinery.

## Quick start: a complete natal chart

```rust
use eternal_astrology::{
    find_aspects, BirthData, ChartConfig, HouseSystem, NatalChart, OrbTable, Zodiac,
};
use eternal_sky::{Body, EphemerisSession, Instant, Observer, SessionConfig};

let session = EphemerisSession::open(SessionConfig::vsop2013())?;

let birth = BirthData::new(
    Instant::from_civil_local(1987, 3, 14, 5, 22, 0.0, -240)?,
    Observer::from_degrees(10.4806, -66.9036, 900.0),
).with_name("Subject A");

let config = ChartConfig {
    house_system: HouseSystem::Placidus,
    zodiac: Zodiac::Tropical,
    ..ChartConfig::default()
};

let chart = NatalChart::compute(&birth, &config, &session)?;

println!("Ascendant: {}", chart.ascendant().to_chart_format());
println!("Midheaven: {}", chart.midheaven().to_chart_format());

for placement in &chart.placements {
    println!("{:>8}  {}  House {:>2}  {}",
        placement.body.name(),
        placement.longitude.to_chart_format(),
        placement.house_number,
        if placement.is_retrograde() { "R" } else { " " },
    );
}

let aspects = find_aspects(&chart, &OrbTable::modern_western());
for a in &aspects {
    println!("{:>10} {:?} {:<10}  orb {:>5.2}°  {}",
        a.a.name(), a.kind, a.b.name(),
        a.orb_abs_deg(),
        if a.applying { "applying" } else { "separating" },
    );
}
# Ok::<_, eternal_astrology::AstrologyError>(())
```

## Forecasting

```rust
use eternal_astrology::*;
use eternal_sky::{Body, Instant};

// Solar Return for 2025:
let natal_sun = chart.placement(Body::Sun).unwrap().longitude.longitude_rad();
let after_birthday = Instant::from_civil_utc(2025, 3, 1, 0, 0, 0.0)?;
let solar_return_2025 = next_return(&session, Body::Sun, natal_sun, after_birthday, None)?;

// Secondary progression at age 30:
let prog = secondary_progression(&chart, &session, 30.0)?;

// Solar arc directions at age 30:
let arc = solar_arc_true(&chart, &session, 30.0)?;

// All primary directions in the first 80 years of life:
let dirs = all_directions(
    &chart,
    DirectionMethod::PlacidusMundane,
    DirectionKey::Naibod,
    80.0,
);

// Current transits to natal:
let now = Instant::from_civil_utc(2026, 5, 15, 12, 0, 0.0)?;
let targets = default_natal_targets(&chart);
let transits = find_current_transits(
    &chart, &session, now,
    &[Body::Mars, Body::Saturn, Body::Jupiter,
      Body::Uranus, Body::Neptune, Body::Pluto],
    &targets,
    &OrbTable::modern_western(),
    AspectKind::MAJORS,
)?;

// Synastry between two charts:
let sync = find_synastry_aspects(
    &chart_a, &chart_b,
    &OrbTable::modern_western(),
    AspectKind::MAJORS,
);
```

## Modules

| Module               | Purpose                                                                 |
|----------------------|-------------------------------------------------------------------------|
| `angles`             | Shared `signed_delta_*`, `wrap_two_pi`, `unsigned_arc_deg` helpers     |
| `birth_data`         | `BirthData` + `TimeCertainty`                                           |
| `chart_config`       | `ChartConfig`, `BodySet`                                                |
| `chart`              | `NatalChart::compute` and accessors                                     |
| `zodiac`             | `Sign` enum, `Zodiac` (Tropical/Sidereal), `SignedLongitude`            |
| `house_system`       | `HouseSystem` enum + `Houses::compute`                                  |
| `placement`          | `BodyPlacement` (sign, house, RA/Dec, derived `is_retrograde()`)        |
| `mundane`            | DA, semi-arcs, Placidus quadrant `m`                                    |
| `aspect`             | `AspectKind`, `OrbTable`, `find_aspects`                                |
| `returns`            | `next_return` (planetary returns)                                       |
| `progression`        | Secondary / Tertiary / Minor progressions                               |
| `solar_arc`          | Solar Arc directions (true / Naibod)                                    |
| `primary_direction`  | Placidus mundane, Regiomontanus, and Campanus directions                |
| `transits`           | Current snapshot + next-exact transit                                   |
| `stations`           | Retrograde / direct station finder                                      |
| `synastry`           | Cross-chart aspect grid                                                 |
| `composite`          | Midpoint composite chart                                                |
| `lots`               | Arabic Parts (Hellenistic Lots) with sect-aware reversal                |
| `profections`        | Annual + monthly profections with traditional / modern rulerships       |
| `lunar_phase`        | 4 canonical phases + 8-fold lunation classification                     |
| `eclipses`           | Solar / lunar eclipse search and on-natal proximity filter              |

## Design

- **Astronomy first.** Every astrology routine forwards to `cosmos-sky` and ultimately to the validated `cosmos-validation::oracle::Oracle`. No parallel ephemerides, no shortcuts.
- **Lazy where it matters.** `BodyPlacement` carries forward longitude rate + RA/Dec from `ApparentPosition`, so the aspect/applying engine and the mundane helpers do not re-query the ephemeris.
- **Interpretation-free.** No body has a "rulership", no aspect has a "meaning". Configure orbs, house systems, ayanamshas and bodies; pattern-match on the results in your own application layer.
- **Reusable primitives.** `find_root` from `cosmos-sky` powers returns, transits, and future timing queries — adding a new "find next X" is ~30 lines.

## License

Licensed under the Apache License, Version 2.0
([LICENSE-APACHE](../LICENSE-APACHE) or
<https://www.apache.org/licenses/LICENSE-2.0>).

## Acknowledgements

This crate was added to the `eternal` workspace by Sergio Velásquez
Zeballos in collaboration with Claude (Anthropic). It builds on the
upstream [celestial](https://github.com/gaker/celestial) project by
Greg Aker and on the validated astronomy of `cosmos-validation`.

### With thanks to

For their guidance, conversations, and inspiration that shaped the
direction of this astrology pipeline:

- **Roberto Reiley**
- **Germán Rosas**
- **Juan Velásquez**
- **Guillermo Velásquez**
