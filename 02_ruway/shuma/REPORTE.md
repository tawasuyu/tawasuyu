# shuma — reporte técnico para IA

> Estado: **2026-05-28** · rama `main` · compila limpio (`cargo build -p shuma-shell-llimphi -p shuma-daemon -p shuma-cli -p shuma-gateway`).
> Audiencia: sesión de Claude futura u otra IA que retome el shell+plugins. Idioma del proyecto: español.

---

## 1. Mapa del subárbol `02_ruway/shuma/`

```
shuma/
├── shuma-cli/              ← CLI admin del daemon (postcard sobre Unix socket)
├── shuma-daemon/           ← runtime: dueño de Workspaces, admin socket, reaper
├── shuma-gateway/          ← bridge HTTP/JSON ↔ postcard (1 endpoint: POST /rpc)
├── shuma-shell-llimphi/    ← CHASIS gráfico (Llimphi) — host de los módulos
├── baremetal/              ← stack de "matilda" (admin server declarativa)
│   ├── matilda-core, -plan, -discover, -apply, -ghost, -linker, -config, -app
└── sandbox/                ← crates de soporte del shell (sync, agnósticos de UI)
    ├── shuma-card          ← Workspace/Pipeline/CommandRef → card_core::Card
    ├── shuma-core          ← runtime in-memory (Mutex<HashMap>), reap, persist
    ├── shuma-protocol      ← wire postcard u32-BE-prefix (daemon ↔ cli/gui)
    ├── shuma-discern       ← discerners (magic-bytes, JSON, TOML, UTF8, Card)
    ├── shuma-exec          ← ejecución sync: Direct / Shell / Pty; eventos mpsc
    ├── shuma-link          ← Noise_XK + identity X25519 + FramedChannel
    ├── shuma-remote-exec   ← cliente sync del ExecStream del daemon
    ├── shuma-line          ← lex/parse/decorate/complete del input (sin frontend)
    ├── shuma-history       ← JSONL append-only + fuzzy (nucleo_matcher)
    ├── shuma-session       ← WorkSession (cwd, runs, grupos)
    ├── shuma-intent        ← grafo de intenciones %cN/%pN
    ├── shuma-shell-render  ← CanvasPlan (lienzo de contexto agnóstico)
    ├── shuma-sysmon        ← /proc/stat + /proc/meminfo + historial
    ├── shuma-module        ← contrato estructural de módulos (sin trait dyn)
    ├── shuma-module-shell      ← MVP REPL (sh -c, sync, builtins)
    ├── shuma-module-matilda    ← admin declarativa como tab del shell
    ├── shuma-module-launcher   ← PLACEHOLDER
    └── shuma-module-commandbar ← PLACEHOLDER
```

---

## 2. Arquitectura en una pantalla

```
                              ┌────────────────────────────┐
   shuma-cli ─postcard──┐     │  shuma-shell-llimphi       │
   shuma-gateway ─json──┤     │  (chasis Llimphi)          │
                        ▼     │   ┌──────────────────────┐ │
                ┌─────────────┴┐  │  Slots:              │ │
                │ shuma-daemon │  │   TopBar  (launcher) │ │
                │  (admin sock)│  │   Main    (matilda…) │ │
                │  + reaper    │  │   Drawer  [shell|…]  │ │
                │  + Workspace │  │   BottomBar (cmdbar) │ │
                │    Manager   │  └──────────────────────┘ │
                └──────┬───────┘             │             │
                       │                     ▼             │
                       │           ┌─────────────────────┐ │
                       │           │ shuma-module-shell  │ │
                       │           │  ↓ (cuando se cablee)│ │
                       │           │  shuma-exec / -line │ │
                       │           │  -history / -session│ │
                       │           └─────────────────────┘ │
                       │           ┌─────────────────────┐ │
                       │           │ shuma-module-matilda│ │
                       │           │  → baremetal/matilda│ │
                       │           └─────────────────────┘ │
                       └────────────────────────────────────┘
                       (vía shuma-protocol — hoy NO cableado
                        desde el shell; el shell ejecuta local
                        con sh -c, no via daemon)
```

**Puntos clave**:
- El chasis es **static-dispatch**: enum `Kind { Launcher, CommandBar, Shell, Matilda }`. Agregar un módulo = variante + ramas en `update`/`view`. Sortea que `llimphi-ui` no tenga `View::map`.
- Cada módulo expone `pub fn make(host) -> ...`; el binario `shuma-shell-llimphi` enlaza estáticamente y mapea `ModuleMsg → ShellMsg` con un cierre (`lift`).
- El daemon, la CLI y el gateway son una **familia paralela** al chasis. El módulo `shuma-module-shell` ejecuta hoy directo con `sh -c` (no habla con el daemon).

---

## 3. Qué está hecho

### 3.1 Chasis gráfico (`shuma-shell-llimphi`, 1 588 LOC)
- **Layout completo**: TopBar, Main, BottomBar, Drawer-Quake (40 % altura por defecto), monitor stack con stat-cards + curvas (CPU, MEM + monitores aportados por módulos).
- **Slots configurables** vía `shumarc-modules.toml` (`src/config.rs`): cualquier `id` no compilado se ignora con warning — el shumarc no rompe el arranque.
- **Drawer**: toggle por F12, cerrar por Esc, click en command-bar abre. *Hover trigger pendiente* (`main.rs:40`: faltan enter/leave events en llimphi-ui).
- **Toolbar de shortcuts** alimentada por `ModuleContributions` (declarativo).
- **Resize del panel de monitores** con drag (splitter).
- **i18n**: `rimay_localize::init()` en `main` — todas las cadenas vía `t("shuma-…")`.

### 3.2 Daemon stack (`shuma-daemon` + `shuma-cli` + `shuma-gateway`)
- **Protocolo** (`shuma-protocol`, 589 LOC): postcard sobre Unix socket; `Request`/`Response` con Workspace CRUD, Run one-shot, Pipeline, ExecStream, Discern, Health, Caps.
- **Daemon** (1 279 LOC): `WorkspaceManager` (Mutex<HashMap>), reap cada 500 ms, drena pipelines en restart, persist a disco, sidecar pool opcional al broker `card_sidecar`.
- **CLI** (`shuma`, 740 LOC): subcomandos `ping`, `health`, `caps`, `workspace {create|list|stop}`, `run`, `commands`, `discern`, `pipeline …`.
- **Gateway HTTP** (168 LOC): `POST /rpc` con body JSON → postcard → daemon. Bind por env `SHIPOTE_GATEWAY_LISTEN`, default `127.0.0.1:7378`. Sin axum/hyper — parser ad-hoc.
- **Noise_XK** (`shuma-link`, ~860 LOC): handshake, `KnownPeers` (allowlist tipo `authorized_keys`), `Keypair` X25519 en `~/.config/shuma/keys/identity.x25519`, `FramedChannel` (length-prefix + chacha20-poly1305). Listo para reemplazar Unix socket por TCP autenticado.
- **Discern** (`shuma-discern`): pipeline configurable (MagicBytes → CardProbe → JsonProbe → TomlProbe → Utf8Probe).

### 3.3 Stack matilda (`baremetal/`)
- **`matilda-core`**: modelo declarativo (Host, Container, VHost, Inventory).
- **`matilda-plan`**: diff inventario actual vs deseado → `Vec<Action>` ordenado.
- **`matilda-discover`**: lee estado real (v1: por nombre — detecta creates y orphans, no cambios de config de un recurso existente).
- **`matilda-config`**: `Container → docker run`, `VHost → server { … }` de nginx. Funciones puras.
- **`matilda-apply`**: `Action → ApplyStep` (archivos + comandos), agnóstico de transporte.
- **`matilda-ghost`**: ejecutor local (`set -e`), reporta `ApplyReport`.
- **`matilda-linker`**: ejecutor SSH (sobre `brahman-ssh-multiplex`), mismo `ApplyReport`.
- **`matilda-app`** (CLI standalone): `matilda example | plan | script | apply | dry-run` local y remoto.
- **`shuma-module-matilda`** (1 120 LOC, **el módulo más completo**): tab del shell con inventario + plan + log + monitor de "pasos pendientes" + 3 shortcuts (Discover/Plan/Dry-run). Soporta `Source::Local` y `Source::Remote { host, user }` con SSH real. Recarga inventario desde el shumarc.

### 3.4 Línea + ejecución sync (sandbox)
Cinco crates listos pero **NO enchufados** al `shuma-module-shell` actual:
- **`shuma-exec`** (PTY incluido): `Exec::{Direct, Shell, Pty}`, eventos por mpsc (`Stdout`/`Stderr`/`Bytes`/`Truncated`/`Spilled`/`Done`), capture-limit + spill a disco, splice(2) zero-copy.
- **`shuma-line`**: tokenize + clasificación, `split_pipeline`, `complete` (con `flag_hints`), `ghost_suggestion`, `decorate_line` (paths clickeables, URLs, grep refs, SHA, `#NN`), `needs_continuation`, parser ANSI completo.
- **`shuma-history`**: JSONL append-only, fuzzy con nucleo_matcher, dedup configurable.
- **`shuma-session`**: WorkSession con cwd, `CommandRun` (estado + salida acotada), grupos guardados.
- **`shuma-shell-render`**: CanvasPlan (lienzo de contexto del grafo de intenciones, agnóstico de UI).
- **`shuma-remote-exec`**: cliente sync del subprotocolo `ExecStream` del daemon — API espejo de `shuma-exec::RunHandle`. Listo para reemplazar `sh -c` por *ejecución contra el daemon*.

### 3.5 Estado actual del REPL (`shuma-module-shell`, ~750 LOC)
- Input + cwd + ejecución **no bloqueante** vía `shuma-exec` (Bloque A1 hecho 2026-05-28). Stdout/stderr llegan en streaming, drenados por `Msg::Tick` que el chasis dispara cada ~100 ms (`SHELL_TICK`). `sleep`/`top`/comandos largos ya no congelan la UI.
- Builtins: `cd`, `pwd`, `clear`, `exit`.
- Tope 500 líneas.
- **Cola de comandos**: si el usuario presiona Enter mientras hay un run vivo, la línea entra en `state.queue` y arranca al cerrar el actual. Header muestra `· ⟳ <cmd> (+N en cola)`.
- **Cancel**: Ctrl-C (o shortcut `Cancel`) manda SIGKILL al grupo de procesos entero — `shuma-exec` lanza cada child con `process_group(0)` para que `killpg` derribe a bash+sleep en un solo golpe.

---

## 4. Wawa — qué hay y qué falta

`wawa-config` (en `shared/wawa-config`) es el **bus de configuración del SO wawa**: archivo JSON canónico (system: `/etc/wawa/config.json`, user: `$XDG_CONFIG_HOME/wawa/config.json`), watcher `notify` sobre ambos paths, atomic save (`tmp + rename`). Sin daemon pub-sub: las apps leen el archivo y se suscriben a cambios. Esto sobrevive a la transición Linux → arje (cuando wawa sea su propio SO, `system_path()` cambia, el resto no).

Forma actual de la config:
```json
{
  "theme_variant": "dark", "accent": "default",
  "lang": "es-PE", "timefmt_24h": true,
  "modules": { "mirada": true, "shuma": true, "chasqui": true,
               "akasha": true, "minga": true, "agora": true }
}
```

**Estado de la integración shuma ↔ wawa: cero.** Ni el chasis ni los módulos importan `wawa-config`. Hoy el chasis lee sólo `shumarc-modules.toml` (config propia, paralela). Falta:
1. Que `shuma-shell-llimphi` suscriba un watcher `wawa-config` y reactive `theme_variant`/`accent`/`lang` sin reinicio.
2. Que el toggle `modules.shuma` controle si el binario arranca / se autodescarga (hoy es decisión externa).
3. Decidir si los slots del shumarc viven dentro del JSON wawa (sección `modules.shuma.{topbar, main, …}`) o quedan en TOML aparte. **Propuesta**: TOML aparte — el JSON wawa es para perfiles de usuario y preferencias, no para topologías de UI por app.

---

## 5. Plan propuesto (priorizado)

### Bloque A — desbloquear el REPL (alto impacto, contenido)
**Objetivo**: que el módulo shell sea utilizable de verdad (no congele con `sleep`, soporte TUI).

A1. ✅ **Cablear `shuma-exec` al `shuma-module-shell`** (hecho 2026-05-28).
- `Command::output()` reemplazado por `shuma_exec::run(&CommandSpec::shell(...))`.
- `Msg::Tick` drena `try_events()`; chasis dispara `Msg::ShellTick` a 100 ms separado del `Msg::Tick` de sysmon (1 Hz).
- Cap de 500 líneas mantenido. Cola de comandos pendientes (`state.queue`).
- `Msg::Cancel` + Ctrl-C → `killer.kill()` (SIGKILL al grupo). `shuma-exec` ahora arma cada child con `process_group(0)` y `killpg` para que el grupo entero caiga junto.

A2. **Cablear `shuma-line` para decoración del output**.
- `decorate_line` sobre cada línea Stdout → spans con kinds (Path, Url, GrepRef, ShaLike).
- Renderizar con colores del theme (paths subrayados, URLs en accent, SHA monoespaciado).

A3. **Completion + ghost**.
- `LineState` reemplaza el `String` actual del input.
- Tab → `complete()` con `CompletionSource` standard (binarios en `$PATH` + paths + flags).
- Render del `ghost_suggestion` en gris detrás del cursor.

A4. **Historial durable**.
- `shuma-history` en `$XDG_DATA_HOME/shuma/history.jsonl`.
- Ctrl-R abre fuzzy search en un sub-panel.

A5. **PTY para TUI fullscreen** (vim/htop/less/claude).
- Cuando el comando ejecutado matchea allowlist (`vim`, `htop`, `less`, …) o el usuario prefija con `:tui`, usar `Exec::Pty`.
- Renderizar bytes crudos con `shuma-shell-render` + parser vt100 (todavía hay que escribir; `shuma-line::ansi` ya cubre 16-color y mucho de ESC[m).

### Bloque B — integrar el daemon como ejecutor (escala)
**Objetivo**: que el shell pueda hablar contra `shuma-daemon` local *o* remoto sin cambiar la API del módulo.

B1. **Switch local/daemon en `shuma-module-shell`**.
- Trait pequeño `Runner` con `start(spec) -> RunHandle` que ambos crates ya implementan (sin renombrar nada, `shuma-exec::RunHandle` y `shuma-remote-exec::RemoteRunHandle` son espejos).
- `Source::Local { via_daemon: bool }`: cuando `true`, ruta por Unix socket.

B2. **Source remoto cableado vía `shuma-link`** (Noise XK ya existe).
- Reemplazar Unix socket por TCP autenticado contra un `shuma-daemon` corriendo en otro host.
- `KnownPeers` controla allowlist; primer arranque genera identity X25519.

B3. **Sidecar broker**: enganchar el sidecar pool del daemon (`card_sidecar::SidecarPool`) para que cada `Workspace` se anuncie al broker. Hoy está armado pero no se publica nada relevante todavía.

### Bloque C — módulos placeholder
C1. **`shuma-module-launcher` real**: leer apps Llimphi instaladas (vía `wawa-config.modules.*` + manifests), grid con íconos, lanzar por `Handle::spawn`.
C2. **`shuma-module-commandbar` real**: parser de intents simple, autocompletado de comandos del shumarc, palette tipo Cmd-P (binding configurable).

### Bloque D — wawa integration
D1. Suscribir watcher `wawa-config` en `shuma-shell-llimphi::main`.
D2. Reaccionar a cambios de `theme_variant`/`accent`/`lang` sin reiniciar (`Theme::for_variant` + `rimay_localize::set_lang`).
D3. Documentar el contrato: el shumarc topología (qué módulos en qué slots) sigue siendo TOML aparte; el JSON wawa es para preferencias visuales y toggle de apps.

### Bloque E — limpieza pendiente
E1. `shuma-daemon/src/main.rs:520` — `audit_request` con uid 0 placeholder; cablear `SO_PEERCRED` real.
E2. Hover trigger del drawer Quake (requiere PR en `llimphi-ui` para enter/leave events).
E3. Parser real de teclas en shumarc (hoy mapeo manual F1..F24 en `main.rs`).
E4. `shuma-shell-render::paint` está agnóstico; cuando llegue el lienzo de contexto al shell hay que crear un módulo nuevo `shuma-module-canvas` que lo consuma.

### Bloque F — features grandes (post-A/B)
F1. Lienzo de contexto: panel adicional que renderice `shuma-intent::SessionGraph` con `shuma-shell-render::CanvasPlan`. El grafo `%cN`/`%pN` ya existe en `shuma-intent`; falta la UI y el parser de intents en la commandbar.
F2. Job control en el módulo shell: `:jobs`, `:term`, `:stop`, `:cont`, sufijo `&` (`shuma-exec` ya soporta multi-run + kill).
F3. Editor multi-línea: `shuma-line::continuation::needs_continuation` ya está; falta cablear al input.

---

## 6. Decisiones de diseño que conviene preservar

1. **Static dispatch sobre trait objects**: `Kind` enum + `ModuleState` enum. Coste: una rama por módulo en `update`/`view`. Beneficio: cada módulo declara su `Msg` propio sin pelearse con `Box<dyn Any>` y sin downcast.
2. **Sync por dentro, async sólo en bordes**: `shuma-exec`/`shuma-remote-exec` son sync (threads + mpsc); el daemon es tokio. El shell es sync — drena eventos en cada `Tick`. No tirar este patrón "porque tokio es lo moderno": Llimphi es sync.
3. **El módulo no depende de `llimphi-ui` desde `shuma-module`**: sólo desde su crate concreto. Esto deja `shuma-module` (el contrato) testeable sin display.
4. **El daemon ignora errores de bind si existe el socket** (`main.rs:30`): asume restart limpio. *Pendiente*: lockfile + check de PID vivo.
5. **El gateway no usa axum**: parser HTTP ad-hoc en ~120 LOC. No agregar axum sólo para "ser idiomático" — un POST único no lo justifica.
6. **Notación de slots** del shumarc: `[topbar]`, `[main]`, `[bottombar]`, `[[drawer.tabs]]`. Mantener — está documentada en `config.rs:1-44`.
7. **`shuma-protocol::DEFAULT_SOCK_NAME = "shuma.sock"`** en `$XDG_RUNTIME_DIR`. No mover.

---

## 7. Trampas conocidas

- **El binario `shuma-shell` GPUI (3.7k LOC) ya no existe** — se borró en `b92b643`. Cualquier referencia a "shuma-shell" en docs viejas es a esa versión. Las features grandes (completion, decoración, historial) viven en sandbox/* sueltas, no en un shell ensamblado.
- **`russh v0.54.5`** dispara warning de future-incompat — no bloquea, llega vía `matilda-linker`.
- **`gpui extinto en gioser** (memoria del proyecto): nada nuevo sobre GPUI. Todo gráfico es Llimphi.
- **El módulo matilda en remoto SÍ ejecuta SSH real** (vía `matilda-linker`/`brahman-ssh-multiplex`); las pruebas reales necesitan un servidor con sshd alcanzable.
- **`shuma-line::decorate` ya hace mucho** (paths clickeables, URLs, SHAs, grep refs) pero ningún consumidor lo usa hoy — fácil ganancia al cablearlo a `shuma-module-shell`.

---

## 8. Ranking de prioridad

| # | Tarea | Ganancia | Costo |
|---|-------|----------|-------|
| ✅ | A1 — cablear `shuma-exec` al módulo shell | desbloquea UX | hecho 2026-05-28 |
| 1 | A2 — decoración con `shuma-line` | shell se siente terminado | bajo |
| 2 | A4 — historial durable | feature core | bajo |
| 3 | A3 — completion + ghost | feature core | medio |
| 4 | D1-D3 — wawa watcher en chasis | tema + idioma live | bajo |
| 5 | B1 — Runner trait + switch local/daemon | habilita remoto | medio |
| 6 | A5 — PTY + parser vt100 | TUI fullscreen | alto (parser) |
| 7 | C1 — launcher real | UX visible | medio |
| 8 | F1 — lienzo de contexto | killer feature pero opcional | alto |

**Recomendación de orden**: A2 → A4 → A3 → D1..D3 → B1 → resto.

---

## 9. Comandos útiles para retomar

```bash
# Compilar todo el subárbol shuma
cargo build -p shuma-shell-llimphi -p shuma-daemon -p shuma-cli -p shuma-gateway

# Probar el chasis (necesita servidor gráfico Llimphi)
cargo run -p shuma-shell-llimphi

# Daemon + CLI rápida
cargo run -p shuma-daemon &
cargo run -p shuma-cli -- health

# Gateway HTTP
SHIPOTE_GATEWAY_LISTEN=127.0.0.1:7378 cargo run -p shuma-gateway

# Estado de los crates sandbox (sin app que los consuma)
wc -l 02_ruway/shuma/sandbox/*/src/*.rs
```

---

*Generado por Claude (Opus 4.7) — `2026-05-27`. Si el plan cambia, actualizá la tabla de la §8 antes de tocar la §3.*
