# gioser-web

Landing page de **GioSer · En el centro, el ser**: chacana animada con
shaders WebGL2, nebulosa procedural y cuatro botones cardinales
(AIRE · FUEGO · TIERRA · AGUA).

## Arquitectura

```
crates/modules/gioser/
├── gioser-geom/        geometría agnóstica de la chacana (20 vértices)
├── gioser-physics/     resorte-amortiguador N-dim crítico-amortiguado
├── gioser-palette/     4 elementos + cosmos en RGB lineal
├── gioser-shaders/     sources GLSL ES 3.00 (FBM cósmico + SDF chacana)
└── gioser-canvas-web/  renderer WebGL2 que compone todo

crates/apps/gioser-web/  cdylib WASM + index.html + styles.css
```

Los cuatro primeros son agnósticos del runtime (compilan en cualquier
target). `gioser-canvas-web` agrega la dependencia de WebGL2 / web-sys.
Cuando exista `yahweh-web`, los agnósticos siguen tal cual y el renderer
se enchufa al runtime equivalente a `yahweh_launcher::launch_app`.

## Cómo se ve

- **Fondo:** vacío violeta-noche con tres capas de FBM (5 octavas) en
  parallax con el mouse + estrellas titilantes + viñeta radial.
- **Chacana:** SDF de la cruz escalonada con outline gaussiano cyan,
  glow ámbar exterior, aro circular envolvente y rayos sutiles
  (calendario andino).
- **Sol central:** gauss + corona, late con sin(t).
- **Tilt físico:** spring-damper sub-crítico (ζ=0.72, 2.2 Hz) que apunta
  hacia el mouse — overshoot suave, settle de ~600 ms.
- **Botones:** DOM real (accesibles, navegables por teclado, deep-link
  por hash) proyectados desde 3D al viewport cada frame.

## Build

Requiere el target `wasm32-unknown-unknown` y `wasm-bindgen-cli`.

```sh
# Una vez (si no los tenés):
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.99

# Build:
cargo build -p gioser-web --release --target wasm32-unknown-unknown

# Generar bindings JS:
wasm-bindgen \
    target/wasm32-unknown-unknown/release/gioser_web.wasm \
    --out-dir crates/apps/gioser-web/pkg \
    --target web

# Servir (cualquier static server vale):
python3 -m http.server -d crates/apps/gioser-web 8080
# → http://localhost:8080/
```

Si usás `rust-toolchain.toml` con Rust del sistema (Artix/Arch), instalá
el target con tu package manager (`pacman -S rust-wasm` o equivalente) o
montá rustup en un perfil aparte.

## Routing

Los `<a href="#aire|fuego|tierra|agua">` apuntan a anchors locales.
Cuando definamos rutas reales (otras páginas, sub-apps, etc.), basta con
cambiar el `href` en `index.html` o interceptar el click desde JS.

## Tests

Los crates agnósticos tienen tests unitarios:

```sh
cargo test -p gioser-geom -p gioser-physics -p gioser-palette
```
