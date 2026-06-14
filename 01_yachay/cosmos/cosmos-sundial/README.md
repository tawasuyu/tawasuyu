# cosmos-sundial

> Sundial: live gnomon shadow + physical dial layout for [cosmos](../README.md).

Two things a sundial needs: the **live shadow** of a gnomon at an instant, and the **layout** to build a physical dial.

- `sundial_reading(tdb, location)` → instantaneous gnomon shadow: azimuth, length (as a ratio of the gnomon height), the Sun's sky position and its hour angle.
- `dial_layout(kind, latitude)` → hour-line angles + style (gnomon edge) elevation for designing a physical dial. Exact gnomonic formulas: horizontal (`tan θ = sin φ · tan H`), vertical facing the equator (`tan θ = cos φ · tan H`), equatorial (uniform `θ = H`, 15°/h). Flags degenerate cases (horizontal at the equator, vertical at the poles).

## API

```rust
use cosmos_sundial::{sundial_reading, dial_layout, hour_line_angle_deg, DialKind};

// Live shadow.
let r = sundial_reading(&tdb, &location);
let shadow = r.shadow_length_for(2.0); // gnomon 2 m tall → shadow in metres

// Designing a horizontal dial at latitude 51.5° N.
let dial = dial_layout(DialKind::Horizontal, 51.5);
let style_elevation = dial.style_height_deg; // = 51.5° (points at the pole)
for line in &dial.hour_lines {
    println!("{} h → {:.2}°", line.local_hour, line.angle_deg);
}

// A single hour line, e.g. the 3 pm line (H = +45°) on a vertical dial.
let theta = hour_line_angle_deg(DialKind::Vertical, 51.5, 45.0);
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-time`](../cosmos-time/README.md), [`cosmos-ephemeris`](../cosmos-ephemeris/README.md)
