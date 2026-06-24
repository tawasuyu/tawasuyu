# churay — instalador/actualizador gráfico de la suite

*Read this in English: [README.md](README.md).*

«churay» (quechua: *poner / instalar*). Instalador estilo Office para tawasuyu
en **cualquier Linux**: elegís apps de un catálogo, clic en Instalar, y quedan
con su `.desktop` en el menú del sistema. Con **actualizador** integrado.

## Decisión de arquitectura (2026-06-24)

Tres ejes que fijaron el diseño:

1. **Lados A + B.** El instalador trae binarios **precompilados** del bundle
   cuando existen (instalación instantánea, tipo Office) y cae a
   `cargo build --release --bin <prog>` cuando no (dev, con el repo presente).
   Misma UI, mismo flujo; lo decide cada unidad según haya o no binario.
2. **Modo sistema (root) + local.** Modo **Sistema** → `/usr/local`, pide root,
   e incluye componentes fuertes como `arje` (init). Modo **Local** → `~/.local`
   sin root, sólo apps. La unidad lleva un `Scope` (`App`/`System`) que decide
   dónde puede ir.
3. **Capa de paquetes: híbrido.** El "costo caro" es la arquitectura de
   paquetes, y `~/hammer` ya tiene un modelo (CAS BLAKE3 + ed25519). En vez de
   acoplar los builds de los dos repos, **vendorizamos** el tipo de hash de
   hammer (`ArtifactHash`, formato `b3:…` idéntico → interop futura bajo el CAS
   unificado del ADR 0007) y usamos los primitivos **propios** de tawasuyu para
   el resto: firma ed25519 vía `agora-core`. hammer-build/overlay (bwrap, zig,
   overlayfs, root) **no** encajan en un instalador de usuario, así que no se
   usan.

## Crates

- **`churay-core`** — motor frontend-agnóstico. Catálogo (desde la única tabla
  de apps del repo, `app-bus`), manifiesto firmado, instalación atómica,
  registro de lo instalado y chequeo de actualizaciones.
  - `catalog` — `suite_catalog()`: las apps `Exec` de `app-bus` + `arje`.
  - `manifest` — `Unit`/`Manifest`/`SignedManifest` (CAS BLAKE3 + ed25519).
  - `install` — `Source` (trait: bundle / build), `install_unit`, `.desktop`,
    `InstallMode`, instalación atómica (`.tmp` + rename), `uninstall_unit`.
  - `state` — `installed.json` en `<prefix>/share/tawasuyu/`.
  - `update` — `check_updates` / `pending_updates`.
  - `hash` — `ArtifactHash` vendorizado de hammer.
  - `repo` — `RemoteRepo` (Source) + `fetch_signed_manifest`: repo remoto
    firmado servido por HTTP; baja binarios **por hash** (`blobs/<hex>`),
    verifica BLAKE3 y los cachea. Transporte `Fetcher` (curl en producción,
    local en tests). Es lo que cierra el actualizador online.
  - bin **`churay-bundle`** — forja el bundle precompilado + manifiesto firmado.
    Emite también `blobs/<hex>`: **un bundle servido por HTTP es un repo remoto**.
  - bin **`churay-cli`** — frente headless (servidores/scripts/CI):
    `list` · `check` · `install <id…>` · `update [<id…>]` · `uninstall <id…>`.
- **`churay-llimphi`** — la GUI (bin `churay`): catálogo con checkboxes por
  cuadrante, selector de modo, progreso por app, pestaña de actualizaciones con
  chequeo contra el repo remoto, botón "Reabrir como root" (`pkexec`).

## Uso

```bash
# instalador (dev: compila lo que elijas desde el workspace)
cargo run -p churay-llimphi

# forjar el bundle precompilado (lado A) y firmarlo
export CHURAY_SIGN_SEED=$(head -c32 /dev/urandom | xxd -p -c64)
scripts/build-tawasuyu-bundle.sh dist/tawasuyu-bundle

# instalar contra un bundle, sin compilar
CHURAY_BUNDLE=$PWD/dist/tawasuyu-bundle cargo run -p churay-llimphi

# servir el bundle como repo remoto y actualizar online (headless)
( cd dist/tawasuyu-bundle && python3 -m http.server 8080 ) &
CHURAY_REPO=http://localhost:8080 churay-cli --local check
CHURAY_REPO=http://localhost:8080 churay-cli --local install cosmos nada
CHURAY_REPO=http://localhost:8080 churay-cli update      # baja lo que cambió
```

Envs: `CHURAY_BUNDLE` (dir del bundle), `CHURAY_WORKSPACE` (raíz para compilar),
`CHURAY_REPO` (repo remoto firmado), `CHURAY_MODE=system|local`,
`CHURAY_SIGN_SEED` (firma del bundle).

Prioridad de fuente al instalar: **bundle local → repo remoto → compilar**.

## Pendiente

- Bundle 100% portable (musl estático / AppImage) para apps GPU. Hoy el bundle
  es dinámico (glibc comparable).
- Confianza anclada por defecto: hoy la firma se autoverifica; falta sembrar la
  clave pública del publicador para exigirla (`SignedManifest::verify(Some(&k))`
  ya lo soporta).
