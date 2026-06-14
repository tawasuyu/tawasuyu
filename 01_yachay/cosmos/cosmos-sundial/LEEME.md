# cosmos-sundial

> Reloj de sol: sombra viva del gnomon + trazado del cuadrante físico para [cosmos](../README.md).

Dos cosas necesita un reloj de sol: la **sombra viva** del gnomon en un instante, y el **trazado** para construir uno físico.

- `sundial_reading(tdb, location)` → sombra instantánea del gnomon: azimut, largo (como múltiplo de la altura del gnomon), posición del Sol y su ángulo horario.
- `dial_layout(kind, latitude)` → ángulos de las líneas horarias + elevación del estilo (filo del gnomon) para diseñar un cuadrante físico. Fórmulas gnomónicas exactas: horizontal (`tan θ = sin φ · tan H`), vertical mirando al ecuador (`tan θ = cos φ · tan H`), ecuatorial (uniforme `θ = H`, 15°/h). Marca los casos degenerados (horizontal en el ecuador, vertical en los polos).

## API

```rust
use cosmos_sundial::{sundial_reading, dial_layout, hour_line_angle_deg, DialKind};

// Sombra viva.
let r = sundial_reading(&tdb, &location);
let sombra = r.shadow_length_for(2.0); // gnomon de 2 m → sombra en metros

// Trazar un cuadrante horizontal a latitud 51.5° N.
let dial = dial_layout(DialKind::Horizontal, 51.5);
let elev_estilo = dial.style_height_deg; // = 51.5° (apunta al polo)
for linea in &dial.hour_lines {
    println!("{} h → {:.2}°", linea.local_hour, linea.angle_deg);
}

// Una línea horaria suelta, p.ej. las 15 h (H = +45°) en cuadrante vertical.
let theta = hour_line_angle_deg(DialKind::Vertical, 51.5, 45.0);
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-time`](../cosmos-time/README.md), [`cosmos-ephemeris`](../cosmos-ephemeris/README.md)
