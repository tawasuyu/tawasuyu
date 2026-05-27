<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# cosmos

> Astronomía cheqaq tupukuywan. Pacha · efemérides · coordenadas · siq'ikuna · astrología.

Rust suite astronómiko yupanapaq, oficial ephemerideswan tupachisqa (JPL DE440/441, IAU 2006/2000A, IERS). Pacha escalakuna (UTC/TT/TAI/UT1)-manta WCS proyecciones-kama, hanaq pacha catálogos, planeta posiciones, eclipsekuna, tránsitos, inti pacha, qucha-pacha, tropikal sideral astrología.

## Churay

```sh
# CLI
cargo run --release -p cosmos-cli -- --help

# Llimphi (hanaq mapa + ephemerides kawsaqkuna)
cargo run --release -p cosmos-app-llimphi

# HTTP server
cargo run --release -p cosmos-server
```

## Tinkuy

- **Linux / macOS / Windows** — `core` crateskuna sistema deps illaqta wiñakun.
- **Wawa** — corekuna WASM-man wiñankun.
- **Web** — `cosmos-web` subset.
- **JPL Horizons** + **AstroPy** validation `cosmos-validation`-pi.

## Crateskuna

Sumaq tabla [README.md](README.md)-pi. Importantekuna: `cosmos-{time,coords,ephemeris,pointing,catalog,sky,wcs,astrology,rise-set,transits,eclipses,sundial,tides,leo}`, hinaspa `cosmos-{cli,server,app-llimphi,web,validation}`.

## Yuyaykunaq

- **Mana runaq sensible-datos hawapi ruwana.** Lat/lon manaña binario manta lloqsinchu mana runaq munaynin.
- DE files **sutilla** wasi-chayasqa `cosmos-cli download`-rayku.
- Astrología t'aqasqa: mana munanki chayqa, `cosmos-astrology` mana huñukuy.
