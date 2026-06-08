# tawasuyu-web

Landing page de **Tawasuyu · En el centro, el ser**: chacana animada con
shaders WebGL2, nebulosa procedural y cuatro botones cardinales
(AIRE · FUEGO · TIERRA · AGUA).

## Arquitectura

```
crates/modules/tawasuyu/
├── tawasuyu-geom/        geometría agnóstica de la chacana (20 vértices)
├── tawasuyu-physics/     resorte-amortiguador N-dim crítico-amortiguado
├── tawasuyu-palette/     4 elementos + cosmos en RGB lineal
├── tawasuyu-shaders/     sources GLSL ES 3.00 (FBM cósmico + SDF chacana)
└── tawasuyu-canvas-web/  renderer WebGL2 que compone todo

web/tawasuyu-web/  cdylib WASM + index.html + styles.css
```

Los cuatro primeros son agnósticos del runtime (compilan en cualquier
target). `tawasuyu-canvas-web` agrega la dependencia de WebGL2 / web-sys.
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

---

## Requisitos

- **Rust** con el target `wasm32-unknown-unknown` instalado.
  - Con `rustup`: `rustup target add wasm32-unknown-unknown`.
  - En Artix/Arch con Rust del sistema: el target suele venir incluido en
    `/usr/lib/rustlib/wasm32-unknown-unknown` (verificá con
    `ls /usr/lib/rustlib | grep wasm`). Si falta: `pacman -S rust-wasm`
    (no existe — el target viene con el paquete `rust` base).
- **wasm-bindgen-cli** (versión exacta 0.2.121, debe matchear la dep del
  `Cargo.lock`). Verificá con
  `grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | head` antes de instalar.

```sh
# Una sola vez:
cargo install wasm-bindgen-cli --version 0.2.121 --locked
```

> La versión del CLI **debe** coincidir con la del crate `wasm-bindgen`
> en `Cargo.lock`. Si no coincide, el output JS no carga el `.wasm`
> generado. Si actualizás el workspace y wasm-bindgen sube de versión,
> reinstalá el CLI con la nueva versión.

- Un static server para probar local: `python3 -m http.server` alcanza.

---

## Flujo rápido (un comando)

Hay un wrapper que hace cargo build + wasm-bindgen + copia salida:

```sh
# Dev — sin optimización, build rápido (~10 s).
./scripts/build-tawasuyu-web.sh dev

# Release — opt-level=3, lto, strip, ~30 s pero binario pequeño.
./scripts/build-tawasuyu-web.sh release
```

El output queda en `web/tawasuyu-web/pkg/`:
```
pkg/
├── tawasuyu_web.js              ← bindings JS (referenciados por index.html)
├── tawasuyu_web_bg.wasm         ← binario WASM
└── tawasuyu_web.d.ts            ← typings (no se usan en runtime, son para IDE)
```

---

## Probarlo local

```sh
./scripts/build-tawasuyu-web.sh dev
python3 -m http.server -d web/tawasuyu-web 8080
# Abrir http://localhost:8080/
```

Si cambiás Rust: re-ejecutar el script y refrescar el browser.
Si cambiás `index.html` / `styles.css`: alcanza con refrescar.

---

## Build release y deploy

```sh
./scripts/build-tawasuyu-web.sh release
```

El binario release optimiza:
- `opt-level = 3`, `lto = "thin"`, `codegen-units = 1`,
  `panic = "abort"`, `strip = "symbols"` (del perfil `[profile.release]`
  del workspace).
- WASM resultante típico: **~120 KB** (canvas + shaders + bindings) sin
  comprimir, **~50 KB** gzippeado. wasm-bindgen pasa por
  `wasm-opt` automáticamente si lo encontrás en `$PATH` (instalable via
  `binaryen`).

Para deploy, los **artefactos a subir al host estático** son sólo
cuatro archivos:

```
web/tawasuyu-web/
├── index.html       ← entry point
├── styles.css       ← estilos
└── pkg/
    ├── tawasuyu_web.js
    └── tawasuyu_web_bg.wasm
```

Funciona en cualquier static host (Nginx, GitHub Pages, S3+CloudFront,
Caddy, netlify, fly, Vercel static). **Importante:**

- El server debe servir `.wasm` con `Content-Type: application/wasm`.
  Nginx/Caddy lo hacen por default; algunos hosts muy viejos no — fijate.
- `index.html` referencia `./pkg/tawasuyu_web.js` con `type="module"`,
  o sea que el browser usa el ES module loader. Eso requiere servir por
  HTTP/HTTPS (no `file://`).
- Fonts de Google: el `<link>` apunta a `fonts.googleapis.com`. Para uso
  offline o sin tracking, descargá las fuentes y servilas locales.

### Comando deploy "tar + scp"

```sh
./scripts/build-tawasuyu-web.sh release
tar czf tawasuyu-web-dist.tar.gz \
    -C web/tawasuyu-web \
    index.html styles.css pkg/

# Subir al server:
scp tawasuyu-web-dist.tar.gz user@host:/var/www/tawasuyu/
ssh user@host 'cd /var/www/tawasuyu && tar xzf tawasuyu-web-dist.tar.gz'
```

### GitHub Pages / gitea pages

Configurá la branch de pages a apuntar a un directorio que contenga
sólo los 4 archivos. Un workflow CI que corra el script y commitee el
`pkg/` a la branch de deploy hace el trabajo.

---

## Routing

Los `<a href="#aire|fuego|tierra|agua">` apuntan a anchors locales.
Cuando definamos rutas reales (otras páginas, sub-apps, etc.), basta con
cambiar el `href` en `index.html` o interceptar el click desde JS.

---

## Tests de los crates agnósticos

Los cuatro crates sin gpui/web tienen tests unitarios estándar:

```sh
cargo test -p tawasuyu-geom -p tawasuyu-physics -p tawasuyu-palette -p tawasuyu-shaders
```

`tawasuyu-canvas-web` no tiene tests (depende de WebGL2 que sólo existe
en browser).

---

## Troubleshooting

**Pantalla en blanco + error en consola "no link/binding found"**
→ Versión de `wasm-bindgen-cli` no coincide con la del `Cargo.lock`.
Reinstalá con `cargo install wasm-bindgen-cli --version <X.Y.Z>`
donde `<X.Y.Z>` sale de `grep '^version' Cargo.lock` cerca de la entrada
`wasm-bindgen`.

**"WebGL2 not supported"** en algunos navegadores viejos
→ No hay fallback. WebGL2 es soporte universal en navegadores modernos
(Chrome/Edge/Firefox/Safari desde 2017). Para targets ancestrales habría
que escribir un renderer WebGL1, no contemplado por ahora.

**Build muy lento por las deps de web-sys**
→ Las features de web-sys están minimizadas en el Cargo.toml; sólo se
importan las que el renderer usa. El primer build sí es lento (~1 min),
los incrementales son rápidos.
