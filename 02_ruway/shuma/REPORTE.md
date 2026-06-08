# shuma — reporte técnico para IA

> Estado: **2026-05-31** · rama `main` · compila limpio (`cargo build -p shuma-shell-llimphi -p shuma-daemon -p shuma-cli -p shuma-gateway`).
> Audiencia: sesión de Claude futura u otra IA que retome el shell+plugins. Idioma del proyecto: español.

## Estado (2026-05-31)

### Hecho

- **Chasis Llimphi completo** (`shuma-shell-llimphi`): slots TopBar/Main/BottomBar +
  tabs, monitores (CPU/MEM) con splitter, i18n vía `rimay_localize`, theme/locale
  vivos desde el bus `wawa-config`.
- **Bloques A–F del roadmap cerrados (2026-05-28/29)**: REPL usable (streaming no
  bloqueante, decoración clickeable del output, `LineState` con completion+ghost,
  historial JSONL+fuzzy, PTY+vt100 con resize dinámico, paste con bracketed paste,
  33/33 tests); daemon como ejecutor (local/daemon/`DaemonTcp` Noise_XK) + sidecar
  al broker; launcher y commandbar reales (Cmd-P con nucleo_matcher); integración
  wawa (watcher + theme/lang live); limpieza (SO_PEERCRED, parser de bindings, lienzo).
- **Lienzo de intenciones shell↔canvas** (2026-05-29): cada `start_run` aparece como
  `%cN` en el `SessionGraph`; nodo verde/rojo según exit; canvas clickeable que
  inserta `%cN`/`%pN` en el cursor del shell.
- **Adiós al Quake-drawer** (2026-05-29): el chasis es app standalone normal (tabs
  siempre visibles); el overlay launcher vive en `pata` (antes en el retirado
  `mirada-launcher-llimphi`).
- **vim como card themeable** (2026-05): PTY con skin app-aware, drag-to-select +
  copia al clipboard, paste con click derecho/medio, iconitos por tipo en paths.
- **Menús** (lote 4): menú principal + menús contextuales en el chasis.
- **Stack matilda** (`baremetal/`): config declarativa multi-host (core/plan/discover/
  config/apply/ghost/linker/app) + `shuma-module-matilda` (tab con SSH real).
- **Refactors regla #1**: split de `shuma-module-shell` (3028 LOC), `shuma-core`
  (1517) y `shuma-shell-llimphi` main (1522) en módulos.

### Pendiente

- **E2 — hover trigger del drawer**: bloqueado por dispatching de pointer enter/leave
  en `llimphi-ui`.
- **Mouse en el PTY**: vt100 ya parsea los eventos; falta cablear el mouse de Llimphi.
- **Tooltip "what would clicking this do?"** en decoraciones (espera al hover de llimphi-ui).
- **Cablear `shuma-line::decorate`** completo desde más consumidores (ya hace mucho,
  poco consumido).
- **Daemon**: lockfile + check de PID vivo (hoy ignora bind si el socket existe).
- **Placeholders residuales**: aunque launcher/commandbar ya tienen impl real, varios
  crates sandbox del listado siguen sin app que los consuma directamente.

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

### 3.5 Estado actual del REPL (`shuma-module-shell`, ~2000 LOC)

**Bloque A completo (2026-05-28).** El REPL ya es una pieza usable.

- **A1** ejecución no bloqueante: streaming via `shuma-exec`, drenado por `Msg::ShellTick` a 100 ms. Cola si hay run vivo. Cancel = SIGKILL al grupo (`process_group(0)` + `killpg`).
- **A2** decoración del output: `shuma_line::decorate_line` por línea; paths/URLs/grep-refs/issue/box-draw → `theme.accent`; git SHAs → `theme.fg_muted`.
- **A3** input inteligente: `LineState` con tokens coloreados, cursor visible, ghost suggestion del historial. Tab completion (binarios en `$PATH` + paths bajo cwd + flag hints + prefijo común con N candidatos). ArrowRight al final acepta ghost. Ctrl+Arrow palabra, Home/End.
- **A4** historial durable: JSONL en `$XDG_DATA_HOME/shuma/history.jsonl`. Up/Down navegan; Ctrl-R abre overlay `fuzzy_search`.
- **A5** PTY + vt100: allowlist + prefijo `:tui` → `Exec::Pty`. `vt100::Parser` alimentado por bytes; render del panel = grid de celdas con `paint_with`. Teclas → xterm bytes.
- **A6** resize dinámico del PTY: `shuma_exec::RunHandle::resize(rows, cols)` expuesto vía `MasterPty` en `Arc<Mutex<>>`; `tui_panel` painter publica el `PaintRect` en `state.last_tui_rect`; cada `drain_run` mira si cambió y manda `MasterPty::resize` + reescala el screen del `vt100::Parser`. vim/htop reciben SIGWINCH y reflowean.
- **A7** click handlers en decoraciones: `Msg::OpenDecoration(DecorationKind)`. Path-dir → cd (más recálculo del `ShellSource`); Path-executable → llena el input con el path; Path-archivo / URL → `xdg-open` detached; GrepRef → `$EDITOR +line file`; GitSha → llena el input con `git show <sha>`. Render del output ahora es `FlexDirection::Row` con un nodo por span (los actionables llevan `on_click`).
- **A8** paste + bracketed paste: Ctrl-V y Shift+Insert leen el clipboard (vía `arboard`). Sin TUI → `LineState::insert`. Con TUI → `RunHandle::write_input`; si el child habilitó bracketed paste (DECSET 2004, leído de `screen.bracketed_paste()`), la secuencia se envuelve en `\x1b[200~…\x1b[201~` para que vim/emacs distingan tipeo de pegado.
- Builtins: `cd`, `pwd`, `clear`, `exit`. Tope 500 líneas en el buffer.
- Tests: **33/33 verde** (timing del ejecutor, navegación de historial, tab/ghost/clicks/paste, build_spec routing, key→PTY bytes, palette ansi, partition_line, decoration handlers, PTY resize end-to-end con `stty size`).

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

**Estado de la integración shuma ↔ wawa (2026-05-28): activa.**

- `shuma-shell-llimphi::init` carga `WawaConfig::load()` + `theme_from_wawa(&wawa, &Theme::dark())` + `rimay_localize::set_locale(&wawa.lang)` antes de armar las instancias, así el primer render ya sale con el theme y locale correctos.
- Un `wawa_config::ConfigWatcher` corre en background; cada cambio dispara `Msg::WawaConfigChanged(Box<WawaConfig>)` vía `Handle::dispatch`.
- El handler re-arma `m.theme` con el nuevo variant/accent (fallback al theme actual si el variant es desconocido) y reinvoca `set_locale` — sin reiniciar el chasis, sin re-cargar las instancias. Los próximos `view()` ya pintan con la paleta nueva; los strings que viajan por `t(...)` también se rehidratan al cambiar.

**Contrato dividido (D3):**

- **`shumarc-modules.toml`** (TOML, project-local): topología de la UI del shell — qué módulo se monta en qué slot (TopBar/Main/BottomBar/Drawer), labels custom, Source (Local/Daemon/DaemonTcp/Remote). Esto es estructura de la app y vive con la app.
- **`$XDG_CONFIG_HOME/wawa/config.json`** (JSON, perfil del usuario): preferencias visuales (`theme_variant`, `accent`), locale (`lang`), formato del reloj (`timefmt_24h`), bitmask de qué apps están on (`modules.{shuma, mirada, pluma, …}`). Esto es preferencia del usuario y es compartida por **todas** las apps Llimphi de tawasuyu (pluma, dominium, cosmos, nada, nakui, shuma…).

El toggle `modules.shuma = false` en el JSON wawa no apaga el binario corriendo (el chasis no se suicida); el efecto es que los launchers no listan a shuma como app activa. La supervisión del binario en sí es decisión del SO (wawa-init en el futuro arje, o systemd/manual hoy).

---

## 5. Plan propuesto (priorizado)

### Bloque A — desbloquear el REPL  ✅ **completo (2026-05-28)**
Ver §3.5 para el detalle del estado actual. Resumen:

- A1 ✅ ejecución no bloqueante + cola + cancel SIGKILL al grupo
- A2 ✅ decoración del output (paths/URLs/SHAs/grep-refs/issue/box-draw)
- A3 ✅ LineState + tokens coloreados + Tab completion + ghost
- A4 ✅ historial durable JSONL + Up/Down + Ctrl-R fuzzy overlay
- A5 ✅ PTY + emulador vt100 (vía `vt100` crate) + render de grid
- A6 ✅ resize dinámico del PTY (`RunHandle::resize` + tracking del PaintRect del panel)
- A7 ✅ click handlers sobre decoraciones (Path/Url/GrepRef/GitSha)
- A8 ✅ paste con bracketed paste (`arboard` + DECSET 2004)

Pendientes opcionales (no bloquean nada):
- Mouse en el PTY (vt100 ya parsea los eventos; falta cablear el mouse de Llimphi).
- Tooltip "what would clicking this do?" en decoraciones (espera al hover en llimphi-ui).

### Bloque B — integrar el daemon como ejecutor ✅ **completo (2026-05-28)**

B1 ✅ **Runner enum local/daemon** en `shuma-module-shell`. `BackendHandle` envuelve `shuma_exec::RunHandle` y `shuma_remote_exec::RemoteRunHandle` con la misma API (`try_events`, `is_finished`, `kill`, `write_input`, `resize` — write/resize son no-op en remoto). `Source` extendido con variantes `Daemon { socket: Option<PathBuf>, label }` y `DaemonTcp { addr, server_pub_hex, label }`; `start_run` rutea según la variante. PTY siempre cae a local con notice (daemon no soporta PTY remoto).

B2 ✅ **Source remoto via Noise XK**. `Source::DaemonTcp` consume `shuma_remote_exec::run_tcp`. Identidad X25519 del shell persiste vía `shuma_link::Keypair::load_or_generate(Keypair::default_path())` — primer arranque genera, después se reusa. `server_pub_hex` parseado con `PublicKey::from_hex`. Errores (no hay daemon, pubkey errónea) salen como notice en el output sin tumbar el shell.

B3 ✅ **Sidecar broker en daemon**. `WorkspaceCreate` ahora llama a `pool.spawn(build_workspace_card(label, id))` cuando hay pool — cada workspace se publica al broker como `Card { kind: Ente, lifecycle: Daemon, flow: ["commands"] }` (paralelo a la `shuma.daemon` card que ya existía). `announce_edges_to_broker` para edges de pipeline ya estaba.

### Bloque C — módulos placeholder ✅ **completo (2026-05-28)**

C1 ✅ **launcher real con manifests**. `shuma-module-launcher` ahora lee `$XDG_CONFIG_HOME/shuma/apps/*.toml` (orden alfabético) en `State::from_apps_dir()`. Cada manifest es `{label, exec?, action_id?}`; si tiene `exec`, click → spawn detached (`process_group(0)`); si no, emite `Msg::EntryClicked(action_id)` al chasis. Si el dir no existe o no hay manifests válidos, cae a `State::demo()` para que el chasis siga exploratorio. Chasis llama a `from_apps_dir()` en lugar de `demo()`.

C2 ✅ **commandbar real Cmd-P**. `shuma-module-commandbar` ahora trae catálogo de `CommandEntry { label, category, kind: FocusTab|Exec|Action }` provisionable vía `State::set_catalog`. Tipear filtra con `nucleo_matcher::Pattern::score`; Up/Down navegan; Enter activa (`activation_for(&state, &ev)` retorna `CommandKind`); Escape limpia; click en row → `ActivateAt(idx)`. Modo `Launcher` usa el catálogo, modo `Shell` ejecuta la línea tal cual (`CommandKind::Exec(text)`). Dropdown se muestra encima de la barra con hasta 8 matches.

### Bloque D — wawa integration
D1. Suscribir watcher `wawa-config` en `shuma-shell-llimphi::main`.
D2. Reaccionar a cambios de `theme_variant`/`accent`/`lang` sin reiniciar (`Theme::for_variant` + `rimay_localize::set_lang`).
D3. Documentar el contrato: el shumarc topología (qué módulos en qué slots) sigue siendo TOML aparte; el JSON wawa es para preferencias visuales y toggle de apps.

### Bloque E — limpieza pendiente
E1 ✅ `audit_request(peer: &str, req)` — Unix socket pasa `uid:1000` desde `SO_PEERCRED`; TCP autenticado pasa `pubkey:<16 hex>` (primeros 16 chars de la X25519 del peer).
E2 ⏳ Hover trigger del drawer Quake — bloqueado por dispatching de `on_pointer_enter/leave` en `llimphi-ui` (WIP del usuario en curso; los campos y métodos públicos existen pero el runtime no los emite todavía).
E3 ✅ Parser real de teclas en shumarc — `parse_binding` acepta `Ctrl+Shift+Space`, `Super+grave`, `Alt+F1`, etc. Modifiers: `Ctrl/Alt/Shift/Super` (con alias `Meta/Cmd/Win`). Named keys: F1..F24, Escape, Enter, Space, Tab, Backspace, Delete, Home, End, PageUp/Down, Arrows, Insert, grave. Tests cubren combos.
E4 ✅ `shuma-module-canvas` consume el `SessionGraph` directo (layout in-tree para no arrastrar `pineal-render` al chasis).

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
- **`gpui extinto en tawasuyu** (memoria del proyecto): nada nuevo sobre GPUI. Todo gráfico es Llimphi.
- **El módulo matilda en remoto SÍ ejecuta SSH real** (vía `matilda-linker`/`brahman-ssh-multiplex`); las pruebas reales necesitan un servidor con sshd alcanzable.
- **`shuma-line::decorate` ya hace mucho** (paths clickeables, URLs, SHAs, grep refs) pero ningún consumidor lo usa hoy — fácil ganancia al cablearlo a `shuma-module-shell`.

---

## 8. Ranking de prioridad

| # | Tarea | Ganancia | Costo |
|---|-------|----------|-------|
| ✅ | A1..A8 — bloque REPL extendido | shell completo | hecho 2026-05-28 |
| ✅ | B1..B3 — daemon ejecutor + broker | shell remoto + observable | hecho 2026-05-28 |
| ✅ | C1..C2 — launcher + commandbar reales | palette Cmd-P + apps | hecho 2026-05-28 |
| ✅ | D1..D3 — wawa watcher + theme/lang live | preferencias unificadas | hecho 2026-05-28 |
| ✅ | E1 E3 E4 — limpieza | SO_PEERCRED, parser bindings, lienzo | hecho 2026-05-28 |
| ✅ | F2 F3 F1 — features grandes | jobs, multi-línea, lienzo | hecho 2026-05-28 |
| ✅ | shell↔canvas live | runs del shell aparecen como `%cN` en el lienzo | hecho 2026-05-29 |
| ⏳ | E2 — hover trigger drawer | requiere WIP llimphi-ui (pointer events) | bloqueado |

**Integración shell↔canvas (2026-05-29).** `shuma-module-shell` mantiene
su propio `SessionGraph` (campo `intent_graph` en `State`) y registra
cada `start_run` como `%cN`. `drain_run` acumula bytes de
stdout/stderr/raw y al cerrar el run llama `complete(id, ok, bytes)` —
nodo verde si `exit 0`, rojo en cualquier otro caso (incluidos errores
de spawn del backend remoto). Builtins (`cd`/`pwd`/`clear`/`exit` y los
`:jobs/:term/:stop/:cont`) no entran al grafo. El chasis añadió
`Kind::Canvas` con tab nuevo "Lienzo" en el drawer por defecto; cada
`SHELL_TICK` (~100 ms) `sync_canvas_from_primary_shell` empuja el grafo
del primer shell encontrado a todas las instancias canvas con
`Msg::SyncGraph(graph)`. El lienzo refleja al instante el flujo de la
sesión (3 tests nuevos en `shell` + 1 en `canvas`).

**Adiós al Quake-drawer (2026-05-29, tercer bloque).** El chasis dejó
de ser una imitación del launcher overlay y volvió a ser app standalone
normal: tabs siempre visibles, sin F12, sin Esc-cierra-drawer, sin
overlay absoluto sobre el escritorio. Eso vive en `pata` (antes en el
retirado `mirada-launcher-llimphi`). Cambios concretos:

- `Model`: `drawer_tabs` → `tabs`, `active_drawer_tab` → `active_tab`,
  fuera `drawer_open` y `drawer_trigger`.
- `Msg`: fuera `ToggleDrawer`, `CloseDrawer`, `SelectDrawerTab` →
  queda `SelectTab(usize)`.
- `Slot::DrawerTab(usize)` → `Slot::Tab(usize)`.
- `on_key` ya no atrapa F12 ni Esc; `forward_key_to_focused_shell`
  prioriza `Slot::Main` y cae al `tabs[active_tab]`.
- `render_main_area` se simplificó: si el shumarc declara `[main]`,
  ocupa todo el área (sin tabs ni monitores). Si no, tabs + splitter
  con monitores a la derecha. No hay más `Position::Absolute` ni
  capas overlay.
- shumarc TOML: `[[drawer.tabs]]` → `[[tabs]]`, fuera `[drawer.trigger]`.
- `parse_binding` + `matches_key` + `tests_bindings` (todo para
  reconocer el shortcut de toggle drawer) borrados.
- i18n: `shuma-empty-no-drawer-tabs`/`-compat` renombrados a
  `shuma-empty-no-tabs`/`-compat`; `shuma-empty-no-main` y el hint que
  mencionaba "F12 abre el drawer" eliminados.

**Canvas clickeable (2026-05-29, segundo bloque).** Las cajas del
lienzo responden al click vía `on_click_at` + `hit_test_box`: el
primer click enfoca el `%cN` (borde 3.5 px en lugar de 2.0), el
segundo desenfoca, y un click en vacío también desenfoca. Cuando hay
un nodo enfocado aparece una tira inferior con la intención completa,
status y bytes; al lado, dos botones "Insertar %cN" / "Insertar %pN"
emiten `Msg::InsertRef(text)`. El chasis intercepta esta variante
(`apply_module_msg` antes de routear al canvas), busca el primer
`Shell` con `first_shell_slot`, abre+enfoca el drawer si está en una
tab, y le manda `Msg::InsertAtCursor(text)` al shell. El shell inserta
en la posición actual del cursor del `LineState`, cierra el overlay
Ctrl-R si estaba abierto y deja el cursor justo después del texto.
`SyncGraph` ahora limpia `focused` si el nodo desapareció del snapshot
nuevo (evita detalle stale). 6 tests nuevos en canvas + 1 en shell.

Pendientes opcionales restantes:
- Mouse en el PTY (vt100 ya parsea; falta cablear el mouse de Llimphi).
- Tooltip "what would clicking this do?" en decoraciones (espera al hover de llimphi-ui).

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
