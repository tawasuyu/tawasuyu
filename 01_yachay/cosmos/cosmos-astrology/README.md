# eternal-astrology

The astrology-specific layer of the `eternal` workspace, built on the [`eternal-sky`](../eternal-sky/) façade.

[![License: Apache 2.0](https://img.shields.io/crates/l/eternal-astrology)](https://gitea.gioser.net/sergio/eternal)

A typed pipeline that turns *(when, where)* into a `NatalChart`: four angles, twelve house cusps in the chosen system, every requested body placed in its sign and house with retrograde flag — plus a full forecasting toolkit (aspects, returns, progressions, solar arc, primary directions, transits, synastry).

## Disclaimer

Astrology is a symbolic system with deep cultural and personal significance for many people. This crate computes its traditional constructs faithfully but **takes no position** on whether those constructs describe, predict, or explain anything about an individual's life. Treat the output as a *language*, not as data. The precision claims in this README refer strictly to the astronomical inputs (planet positions, time scales, IAU rotations); they say nothing about the validity of the astrological interpretations the user may build on top.

## Installation

```toml
[dependencies]
eternal-astrology = "0.1"
eternal-sky = "0.1"
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
| Planetary returns (Sol/Luna/anyone) | `next_return` | ✅ |
| Progressions: Secondary, Tertiary, Minor | `secondary_progression`, … | ✅ |
| Solar Arc directions (TrueProgressedSun, Naibod) | `solar_arc_true`, `solar_arc_naibod` | ✅ |
| Primary directions (Placidus mundane; Ptolemy / Naibod keys) | `direct`, `all_directions` | ✅ |
| Transits — current and next exact | `find_current_transits`, `find_next_exact_transit` | ✅ |
| Synastry — cross-aspects between two charts | `find_synastry_aspects` | ✅ |
| Event root-finder over time (generic) | `eternal_sky::find_root` | ✅ |

61 tests gate the precision and behaviour of these features against direct calls into the validated underlying machinery.

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

| Module               | Purpose                                                      |
|----------------------|--------------------------------------------------------------|
| `birth_data`         | `BirthData` + `TimeCertainty`                                |
| `chart_config`       | `ChartConfig`, `BodySet`                                     |
| `chart`              | `NatalChart::compute` and accessors                          |
| `zodiac`             | `Sign` enum, `Zodiac` (Tropical/Sidereal), `SignedLongitude` |
| `house_system`       | `HouseSystem` enum + `Houses::compute`                       |
| `placement`          | `BodyPlacement` (sign, house, retrograde, RA/Dec)            |
| `mundane`            | DA, semi-arcs, Placidus quadrant `m`                         |
| `aspect`             | `AspectKind`, `OrbTable`, `find_aspects`                     |
| `returns`            | `next_return` (planetary returns)                            |
| `progression`        | Secondary / Tertiary / Minor progressions                    |
| `solar_arc`          | Solar Arc directions (true / Naibod)                         |
| `primary_direction`  | Placidus mundane directions                                  |
| `transits`           | Current + next-exact transit                                 |
| `synastry`           | Cross-chart aspect grid                                      |

## Design

- **Astronomy first.** Every astrology routine forwards to `eternal-sky` and ultimately to the validated `eternal-validation::oracle::Oracle`. No parallel ephemerides, no shortcuts.
- **Lazy where it matters.** `BodyPlacement` carries forward longitude rate + RA/Dec from `ApparentPosition`, so the aspect/applying engine and the mundane helpers do not re-query the ephemeris.
- **Interpretation-free.** No body has a "rulership", no aspect has a "meaning". Configure orbs, house systems, ayanamshas and bodies; pattern-match on the results in your own application layer.
- **Reusable primitives.** `find_root` from `eternal-sky` powers returns, transits, and future timing queries — adding a new "find next X" is ~30 lines.

## License

Licensed under the Apache License, Version 2.0
([LICENSE-APACHE](../LICENSE-APACHE) or
<https://www.apache.org/licenses/LICENSE-2.0>).
