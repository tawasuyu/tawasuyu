# Módulos Llimphi — contrato

Un **módulo Llimphi** es una crate Rust que empaqueta una *feature
funcional completa* (estado + lógica + UI + atajos) de forma que cualquier
app pueda enchufarla sin acoplarse al módulo más allá de lo que el
contrato declara.

Esto es distinto de un **widget** (también vive en `02_ruway/llimphi/`):
los widgets son puramente visuales y reactivos (botón, lista, splitter).
Un módulo encapsula un comportamiento con estado propio y un flujo de
eventos completo — el ejemplo canónico es find-in-files
(`llimphi-module-fif`), pero el mismo patrón sirve para command palette,
diff viewer, mini-map, picker, etc.

## Tier en el repo

```
02_ruway/llimphi/
├── llimphi-{hal,raster,layout,text,theme,ui}   ← framework
├── widgets/                                     ← visuales reactivos
│   ├── tabs/
│   ├── tree/
│   └── …
└── modules/                                     ← features completas
    └── fif/                                     ← (este es el primero)
```

## Forma del contrato

Cada módulo `llimphi-module-X` expone:

| Símbolo            | Rol                                                                                                  |
|--------------------|------------------------------------------------------------------------------------------------------|
| `pub struct XState`| Estado interno. El host lo embebe en su `Model`, típicamente como `Option<XState>` (panel abierto/cerrado). |
| `pub enum XMsg`    | Vocabulario interno del módulo. El host lo wrapea en su `AppMsg` como `AppMsg::X(XMsg)`.            |
| `pub enum XAction` | Efecto que el módulo le pide al host después de procesar un mensaje. Variantes típicas: `None`, `Close`, `OpenAt {…}`, `SetStatus(s)`, etc. |
| `pub fn apply(&mut XState, XMsg, &Ctx) -> XAction` | El reducer puro del módulo. Toma una referencia al contexto que necesita del host (e.g. lista de paths) y muta su propio estado. **No** toca el modelo del host. |
| `pub fn on_key(&XState, &KeyEvent) -> Option<XMsg>` | Routing de teclas cuando el panel está abierto. Devuelve `Some(msg)` si el módulo intercepta el evento, `None` si el host debe seguir su routing normal. |
| `pub fn open_shortcut(&KeyEvent) -> bool` | Predicado que reconoce el atajo de apertura recomendado. El host puede usarlo o definir el suyo. |
| `pub fn view<HostMsg, F>(&XState, …, palette, to_host: F) -> View<HostMsg> where F: Fn(XMsg) -> HostMsg + Copy + 'static` | Render del panel, parametrizado sobre el `Msg` de la app via callback. |
| `pub struct XPalette { … }` + `XPalette::from_theme(&Theme)` | Paleta visual derivable del theme global. |

## Por qué Action en lugar de un trait `XHost`

El loop tipo Elm de Llimphi mueve el `Model` por value en `update(model,
msg)`. Pasarle `&mut Host` al módulo arrastra problemas de borrowing
(simultáneamente quiero `&mut model.x_state` y `&mut model.rest_del_model`).

Devolver una `XAction` corta el nudo: el módulo no sabe *cómo* se ejecuta
el efecto, sólo *qué* efecto desea. El host puede aplicarlo en cualquier
orden, combinarlo con otros side effects, o ignorarlo.

## Por qué NO un trait `LlimphiModule`

Los `XMsg`, `XAction` y signatures de `apply`/`view` varían demasiado
entre módulos para que un trait genérico sea útil sin volverse abstracto
hasta lo inservible. **La convención es el contrato** — la consistencia
está en los nombres y la forma, no en una jerarquía de tipos.

Si en el futuro emerge un patrón que sí justifica un trait (ej. para
serialización de estado, hot-reload, descubrimiento dinámico), se
introduce ahí, no preventivamente.

## Cómo enchufa una app: ejemplo `gioser-edit` ↔ `llimphi-module-fif`

```rust
use llimphi_module_fif::{self as fif, FifAction, FifMsg, FifPalette, FifState};

struct Model {
    all_files: Vec<PathBuf>,
    fif: Option<FifState>,
    // … resto …
}

enum Msg {
    Fif(FifMsg),
    // … resto …
}

// update:
Msg::Fif(fm) => {
    let mut m = model;
    if matches!(fm, FifMsg::Open) && m.fif.is_none() {
        m.fif = Some(FifState::new());
        return m;
    }
    let action = match m.fif.as_mut() {
        Some(s) => fif::apply(s, fm, &m.all_files),
        None => return m,
    };
    match action {
        FifAction::None => {}
        FifAction::Close => m.fif = None,
        FifAction::Searched { matches, elapsed, query } => {
            m.status = format!("«{query}» · {matches} · {:.0} ms",
                               elapsed.as_secs_f64() * 1000.0);
        }
        FifAction::OpenAt { path, line, col } => {
            m.fif = None;
            m = open_path(m, path);
            if let Some(tab) = m.active_tab_mut() {
                tab.editor.set_caret_at(line, col);
            }
        }
    }
    m
}

// on_key:
if let Some(state) = model.fif.as_ref() {
    if let Some(fm) = fif::on_key(state, event) {
        return Some(Msg::Fif(fm));
    }
}
if fif::open_shortcut(event) {
    return Some(Msg::Fif(FifMsg::Open));
}

// view:
if let Some(state) = model.fif.as_ref() {
    let panel = fif::view(
        state, &model.all_files, &model.root,
        &FifPalette::from_theme(&theme),
        Msg::Fif,
    );
    children.push(panel);
}
```

Lo que el módulo gana:
- No conoce `Model` ni `Msg` del host.
- No abre archivos él mismo — pide `OpenAt` y el host elige qué significa
  abrir en su contexto (un tab nuevo, un split, un buffer in-memory).

Lo que el host gana:
- ~300 líneas de UI + state + lógica de búsqueda fuera del binario.
- Reutilización gratis: cualquier otra app (un dominium-explorer, un
  chasqui-broker-explorer, un pluma-app) puede sumar find-in-files con
  ~15 líneas de glue.

## Relación con el protocolo Brahman Card

`card_core::Card` describe **entidades runtime** (procesos con
`payload`/`soma`/`supervision`, gestionados por Init/Admin/Sidecar). Los
módulos Llimphi son **crates de librería** que se linkean al host — no
tienen proceso propio. Por eso un módulo individual **no lleva Card**.

Sí son relevantes en dos lugares:

1. **Card del host**. Cuando una app construye su Card (ej.
   `cosmos_card::build_card()`), puede agregar a `provides` las
   capabilities que sus módulos embebidos aportan. Cada módulo expone
   `pub const CAPABILITIES: &[&str]` con strings tipo
   `"editor.find-in-files"` que el host enrola en su Card antes de
   `spawn_sidecar()`. Beneficio: el broker chasqui descubre que la
   instancia ofrece esas funciones sin que el host tenga que enumerarlas
   a mano.

2. **Plugins WASM runtime (Tier 2)**. Son entidades runtime cargadas
   dinámicamente, con Card completa, `Permissions` enumerados,
   sandboxing real y descubrimiento por broker. Vea
   [§Tier 2 — Plugins WASM](#tier-2--plugins-wasm) más abajo.

## Tier 2 — Plugins WASM

Los módulos Tier 1 (sección anterior) son **crates Rust** que el host
linkea en build-time. Son rápidos, type-safe, y baratos — pero requieren
recompilar el host para agregarlos.

Los **plugins Tier 2** invierten esos trade-offs: son `.wasm` cargados en
runtime, sin recompilar nada. Pierden algo de performance y la
type-safety se vuelve dinámica, pero ganan tres cosas:

1. **Distribución independiente**. Un plugin se entrega como un blob
   (`.wasm` + `manifest.toml`) que cualquier host Llimphi puede consumir.
2. **Sandboxing real**. Lo que un plugin puede hacer está limitado por
   `card_core::Permissions`, no por el código fuente del plugin. Un
   plugin con `filesystem = "none"` físicamente no puede leer disco
   aunque lo intente — el host import no existe en su `Linker`.
3. **Descubrimiento dinámico**. Cada plugin declara `capabilities` en su
   manifest; el `PluginHost` indexa por capability y enruta invocaciones.
   El broker chasqui ve esas capabilities como parte de la Card del host.

### El contrato

Un plugin Tier 2 es **un `.wasm` + un `manifest.toml` hermano**:

```toml
# manifest.toml
name = "hello-status"
version = "0.1.0"
capabilities = ["status.greet"]

[permissions]
networking = "none"     # none | loopback | outbound | full
filesystem = "none"     # none | read-only | read-write
processes  = false

[permissions.ipc]
allow = []              # protocolos IPC permitidos (vacío = sin IPC)
```

El `.wasm` debe exportar:

| Export                                                          | Rol                                                                                                                                |
|-----------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------|
| `memory` (lineal, exportada como `"memory"`)                    | Buffer que el host usa para leer strings que el plugin emite.                                                                       |
| `_invoke(cap_ptr: i32, cap_len: i32, arg_ptr: i32, arg_len: i32) -> i32` | Entry point único. `cap_*` apunta al nombre de capability invocada, `arg_*` al payload (bytes opacos). El retorno es un exit code informativo. |

**ABI de memoria v0**: el host escribe `cap` y `args` empezando en
offset `0` de la `memory` lineal del plugin. Por convención, las `data`
sections del plugin deben vivir por encima del offset `256` para no ser
sobrescritas. Esta restricción es temporal — desaparecerá cuando el
contrato exija exportar `_alloc(len) -> ptr` y el host pida espacio al
plugin antes de escribir.

El host expone como imports (todos bajo el namespace `"plugin"`):

| Import                                  | Permiso requerido                | Efecto                                                                                                |
|-----------------------------------------|----------------------------------|-------------------------------------------------------------------------------------------------------|
| `plugin.log(ptr, len)`                  | siempre disponible               | Traza UTF-8 vía `tracing::info!`. Útil para debug.                                                    |
| `plugin.set_status(ptr, len)`           | siempre disponible               | Emite `PluginAction::SetStatus(s)`. El host típicamente lo pinta en la barra inferior.                |
| `plugin.open_at(path_ptr, path_len, line, col)` | `filesystem >= read-only`        | Emite `PluginAction::OpenAt { path, line, col }`. Si el permiso falta, el import **no se enlaza** y el plugin trap-ea al intentar usarlo. |

La lista crecerá; el principio es: **cada nuevo import se gatea por un
`Permissions` field**, no se agrega "por defecto".

### Por qué Action (igual que Tier 1)

Las invocaciones devuelven `PluginAction` por exactamente la misma razón
que un módulo Tier 1: el plugin no sabe — ni necesita saber — cómo se
"abre" un path en este host, qué significa "set status" en este chrome
visual, o si la app está en modo headless. Devuelve intención; el host
decide ejecución. Esto también desacopla testing: el host puede ejercitar
un plugin sin renderizar nada.

### Por qué un manifest sidecar y no una sección WASM custom

Las custom sections en `.wasm` requieren un toolchain especializado para
escribirlas y otro para leerlas. Un `.toml` hermano se lee con
`serde` (que el workspace ya usa) y se edita a mano si hace falta. La
puerta para mover el manifest a una custom section queda abierta cuando
haya un cargador uniforme — por ahora, pragmatismo gana.

### Ejemplo: cargar e invocar un plugin

```rust
use llimphi_plugin_host::{PluginHost, PluginAction};

let mut host = PluginHost::new();
let id = host.load_from_dir("./plugins/hello-status")?;
// El manifest declara capability "status.greet".

let action = host.invoke(id, "status.greet", b"mundo")?;
match action {
    PluginAction::SetStatus(s) => model.status = s,
    PluginAction::OpenAt { path, line, col } => { /* … */ }
    PluginAction::None => {}
}
```

Si el plugin pidió `status.greet` pero tiene `filesystem = "none"` y su
código llama `plugin.open_at(…)`, el plugin trap-ea inmediatamente
porque ese import no fue enlazado — el host devuelve
`PluginError::Trap` y el `PluginAction` no se emite. La capability *no*
es lo que autoriza; **los permisos sí**.

### Relación con `arje-wasm`

`arje-wasm` (`03_ukupacha/arje/runtime/arje-wasm`) encarna
`Payload::Wasm` como un **Ente del grafo**: un hilo dedicado, ciclo de
vida atado al kernel arje, imports bajo namespace `"ente"`. Esa es la
ruta para *procesos* Wasm de larga vida, supervisados por arje, parte
del grafo dominium.

`llimphi-plugin-host` es la ruta inversa: invocación **on-demand desde
la UI**, sin hilo dedicado, sin Card de proceso (el plugin **no es un
Ente** — vive dentro del proceso del host). Comparten engine wasmi 1.0
para que un mismo `.wasm` *podría* correr en ambos modos si su
contrato lo permite, pero los entry points y namespaces de imports son
distintos a propósito.

## Módulos existentes

| Crate                          | Capability               | Atajo recomendado |
|--------------------------------|--------------------------|-------------------|
| `llimphi-module-fif`           | `editor.find-in-files`   | Ctrl+Shift+F      |
| `llimphi-module-file-picker`   | `editor.file-picker`     | Ctrl+P            |

| Crate (Tier 2 runtime)         | Rol                                                                              |
|--------------------------------|----------------------------------------------------------------------------------|
| `llimphi-plugin-host`          | Carga `.wasm` + `manifest.toml`, sandbox por `card_core::Permissions`, `PluginAction`. |

## Siguientes módulos candidatos

- `llimphi-module-command-palette` — Ctrl+Shift+P estilo VS Code.
- `llimphi-module-diff-viewer` — visualización side-by-side de cambios.
- `llimphi-module-mini-map` — overlay de minimap del buffer activo.
- `llimphi-module-symbol-outline` — outline del documento via LSP
  `documentSymbol`.

Cada uno debería seguir el mismo contrato sin inventar uno nuevo.
