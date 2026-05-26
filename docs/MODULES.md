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

## Siguientes módulos candidatos

- `llimphi-module-file-picker` — el Ctrl+P fuzzy file picker (hoy inline
  en `gioser-edit`).
- `llimphi-module-command-palette` — Ctrl+Shift+P estilo VS Code.
- `llimphi-module-diff-viewer` — visualización side-by-side de cambios.
- `llimphi-module-mini-map` — overlay de minimap del buffer activo.

Cada uno debería seguir el mismo contrato sin inventar uno nuevo.
