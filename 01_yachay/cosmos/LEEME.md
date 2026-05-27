# cosmos

> Astronomía con precisión astronómica. Tiempo · efemérides · coordenadas · imágenes · astrología.

Suite Rust de cálculo astronómico validada contra ephemerides oficiales (JPL DE440/441, IAU 2006/2000A, IERS). Cubre desde escalas de tiempo (UTC/TT/TAI/UT1) hasta proyecciones WCS pasando por catálogos estelares, posiciones planetarias, eclipses, tránsitos, reloj de sol, mareas, astrología tropical y sideral.

## Instalación

```sh
# CLI
cargo run --release -p cosmos-cli -- --help

# App Llimphi (mapa del cielo + ephemerides interactivas)
cargo run --release -p cosmos-app-llimphi

# Server HTTP
cargo run --release -p cosmos-server
```

## Compatibilidad

- **Linux / macOS / Windows** — todos los crates `core` compilan sin dep de sistema.
- **Wawa** — los core compilan a WASM (`cosmos-core`, `cosmos-time`, `cosmos-coords`, ...).
- **Web** — `cosmos-web` expone subset por WASM/JS.
- Validación contra **JPL Horizons** y **AstroPy** en `cosmos-validation`.

## Crates

| Crate | Rol |
|---|---|
| [`cosmos-core`](cosmos-core/README.md) | Tipos base; sin gráficos. |
| [`cosmos-time`](cosmos-time/README.md) | Escalas de tiempo IAU + ΔT histórico. |
| [`cosmos-coords`](cosmos-coords/README.md) | Transformaciones de coordenadas. |
| [`cosmos-ephemeris`](cosmos-ephemeris/README.md) | Posición planetaria via JPL DE. |
| [`cosmos-pointing`](cosmos-pointing/README.md) | Reducción topocéntrica (paralaje, refracción). |
| [`cosmos-catalog`](cosmos-catalog/README.md) | Catálogos estelares (HIP/Tycho/Gaia). |
| [`cosmos-sky`](cosmos-sky/README.md) | Fachada ergonómica (`Instant`/`Observer`/`EphemerisSession`). |
| [`cosmos-wcs`](cosmos-wcs/README.md) | World Coordinate System (FITS-compatible). |
| [`cosmos-images`](cosmos-images/README.md) | Carga + display de imágenes astronómicas (FITS). |
| [`cosmos-astrology`](cosmos-astrology/README.md) | Astrología tropical y sideral. |
| [`cosmos-rise-set`](cosmos-rise-set/README.md) | Salida/puesta de astros. |
| [`cosmos-transits`](cosmos-transits/README.md) | Tránsitos planetarios. |
| [`cosmos-eclipses`](cosmos-eclipses/README.md) | Eclipses solares/lunares. |
| [`cosmos-sundial`](cosmos-sundial/README.md) | Reloj de sol; tiempo aparente local. |
| [`cosmos-tides`](cosmos-tides/README.md) | Mareas (modelo simplificado luna+sol). |
| [`cosmos-skywatch`](cosmos-skywatch/README.md) | Observación general (constelaciones visibles, mejor hora). |
| [`cosmos-leo`](cosmos-leo/README.md) | Órbitas LEO (TLE). |
| [`cosmos-corpus`](cosmos-corpus/README.md) | Corpus textual astronómico ([GUIA](cosmos-corpus/GUIA.md)). |
| [`cosmos-model`](cosmos-model/README.md) | Tipos modelo compartidos. |
| [`cosmos-modules`](cosmos-modules/README.md) | Registro de módulos. |
| [`cosmos-engine`](cosmos-engine/README.md) | Engine genérico de cálculo. |
| [`cosmos-render`](cosmos-render/README.md) | Render agnóstico (skymap + 3D). |
| [`cosmos-canvas-llimphi`](cosmos-canvas-llimphi/README.md) | Backend Llimphi (vello). |
| [`cosmos-app-llimphi`](cosmos-app-llimphi/README.md) | App escritorio. |
| [`cosmos-card`](cosmos-card/README.md) | Card resumen para escritorio. |
| [`cosmos-cli`](cosmos-cli/README.md) | CLI. |
| [`cosmos-store`](cosmos-store/README.md) | Cache local (DE files, catálogos). |
| [`cosmos-server`](cosmos-server/README.md) | HTTP server (REST). |
| [`cosmos-validation`](cosmos-validation/README.md) | Regression harness vs Horizons/AstroPy. |
| [`cosmos-web`](cosmos-web/README.md) | Bindings WASM. |

## Consideraciones

- **Cero ejecución cliente con datos sensibles del usuario.** Latitud/longitud nunca dejan el binario sin permiso.
- Los DE files se descargan **explícitamente** vía `cosmos-cli download`.
- Astrología es separable: si no la querés, no enlazás `cosmos-astrology`.
