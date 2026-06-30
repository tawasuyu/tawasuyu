//! `nahual-shell-llimphi` — MVP del shell nahual sobre Llimphi.
//!
//! Composición mínima: barra superior con la ruta + split draggable
//! con `nahual-file-explorer-llimphi` a la izquierda y
//! `nahual-text-viewer-llimphi` a la derecha. Foco en validar la
//! composición Llimphi y consumir crates reusables; no en paridad con
//! el shell GPUI.
//!
//! Lo que **sí** hace este MVP:
//! - Navegación con teclado: ↑/↓ y rueda mueven la selección/scroll;
//!   Enter entra a un directorio o abre un archivo; Backspace sube al
//!   padre.
//! - Click en una fila: selecciona; si es archivo, lo previsualiza.
//! - Preview de archivos texto pequeños (delegado al crate
//!   `nahual-text-viewer-llimphi`, ≤ 256 KB, UTF-8 sin null bytes).
//! - Splitter draggable.
//!
//! El viewer se elige por **contenido**, no por extensión:
//! `viewer_registry::pick` despacha el `Discernment` de `shuma-discern`
//! (magic-bytes, JSON/TOML/Card probe, UTF-8) al visor que sabe pintar
//! esa naturaleza de dato. Es el germen del "open-with universal":
//! cuando lleguen más visores y un AppBus con `EntityType`, el registro
//! crece por tabla sin tocar el resto del shell.
//!
//! Hoy embebe once visores in-process — texto (fallback universal),
//! imagen, video (AV1 nativo), audio (WAV/MP3/FLAC/Opus/Vorbis por cpal,
//! con espectro en vivo), card (`shared/card` presentada por campos),
//! tree (árbol JSON/TOML indentado), hex (dump de binarios), table
//! (CSV/TSV alineado), markdown (`.md` renderizado con encabezados,
//! listas, código y citas), archive (listado de ZIP/tar/tar.gz; ZIP
//! cubre .jar/.apk/.epub/OOXML) y font (TTF/OTF: metadatos + muestra
//! dibujada con los contornos de la propia fuente) — todos ruteados por
//! `viewer_registry::pick` sobre el `lens`/`mime` discernido. `Space`
//! hace play/pausa del video o audio.
//!
//! Lo que **todavía** no:
//! - `layout.json` / `Persister` / hot-reload.
//! - Otros containers (Tabs, Tiled) y un reader PDF nativo.
//! - AppBus: el viewer recibe el path directo desde el modelo. Cuando
//!   tengamos un bus, el shell publica `EntitySelected` y los viewers
//!   se suscriben.
//!
//! El código está repartido en módulos hermanos (split 2026-06-12):
//! `modelo` (tipos + estado + `Msg` + impls), `update` (`shell_update`),
//! `view` (`shell_view`/`shell_view_overlay` + render), `overlays`
//! (menús + modales) y `helpers` (utilidades). `main.rs` es sólo el
//! chasis `App`.

use std::collections::HashMap;
use std::path::PathBuf;

mod ops;
mod state;
mod viewer_registry;
mod monad_dispatch;
mod monad_icon;
mod modelo;
mod helpers;
mod overlays;
mod palette;
mod find;
mod ai;
mod view;
mod update;

use llimphi_theme::Theme;
use llimphi_motion::Tween;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use llimphi_module_command_palette::{self as command_palette, PaletteMsg};
use app_bus::AppRegistry;
use tullpu_module as tullpu;
use wawa_config_llimphi::theme_from_wawa;

use crate::modelo::*;
use crate::helpers::*;
use crate::overlays::app_menu;

fn main() {
    llimphi_ui::run::<Shell>();
}

struct Shell;

impl App for Shell {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "nahual · shell"
    }

    fn initial_size() -> (u32, u32) {
        (1200, 800)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        // El primer argumento, si es un directorio, fija el cwd de arranque
        // (lo usa `app_bus::reveal` para "Reveal in nahual <dir>").
        let cwd = std::env::args()
            .nth(1)
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("/"));
        let cfg = wawa_config::WawaConfig::load();
        // i18n: cargar catálogos y fijar el locale del usuario (cfg.lang) antes
        // de armar cualquier vista — así los strings de UI salen traducidos.
        rimay_localize::init();
        let _ = rimay_localize::set_locale(&cfg.lang);
        let theme = theme_from_wawa(&cfg, &Theme::dark());
        let handle_clone = handle.clone();
        let watcher = wawa_config::ConfigWatcher::spawn(move |new_cfg| {
            handle_clone.dispatch(Msg::WawaConfigChanged(Box::new(new_cfg)));
        })
        .map_err(|e| eprintln!("nahual-shell · wawa-config watcher: {e}"))
        .ok();
        // Los visores con transporte (video, audio) necesitan un reloj
        // externo: cada pulso avanza un frame / refresca el espectro. Es
        // barato cuando el panel no avanza (el update sale temprano).
        handle.spawn_periodic(FRAME_TICK, || Msg::Tick);
        // El sidebar arranca con el árbol descolapsado hasta el cwd, para que la
        // carpeta actual se vea de entrada.
        let tree_expanded = ancestors_set(&cwd);
        let mut tree_children = HashMap::new();
        ensure_children_for_expanded(&mut tree_children, &tree_expanded);
        Model {
            // Una sola sesión al arrancar; la activa lleva `snap: None`.
            sessions: vec![Session { name: session_name(&cwd), snap: None }],
            active: 0,
            tree_expanded,
            tree_scroll: 0,
            tree_children,
            tree_w: 230.0,
            preview_w: 420.0,
            viewer_open: false,
            tools_open: false,
            tools_w: 280.0,
            wheel_mode: WheelMode::Zoom,
            win: {
                let (w, h) = Self::initial_size();
                (w as f32, h as f32)
            },
            canvas: None,
            clipboard: ShellClipboard::new(),
            last_click: None,
            canvas_drag: (0.0, 0.0),
            // Ambos paneles arrancan en el cwd POSIX; el 1 se ve sólo en dual.
            panes: [
                Pane {
                    nav_stack: vec![posix_nav(&cwd)],
                    marked: std::collections::BTreeSet::new(),
                    hist: vec![cwd.clone()],
                    hist_pos: 0,
                },
                Pane {
                    nav_stack: vec![posix_nav(&cwd)],
                    marked: std::collections::BTreeSet::new(),
                    hist: vec![cwd.clone()],
                    hist_pos: 0,
                },
            ],
            focus: 0,
            dual: false,
            list_width: 400.0,
            nav_filtering: false,
            preview: PreviewPane::Empty,
            preview_of: None,
            preview_temp: None,
            theme,
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            context_menu: None,
            map_view: nahual_map_viewer_llimphi::MapView::default(),
            basemap: None,
            basemap_dirty: false,
            last_restream: None,
            _wawa_watcher: watcher,
            registry: AppRegistry::with_defaults(),
            ctx_open_with: Vec::new(),
            ctx_target: None,
            ctx_temp: None,
            queue: ops::OpQueue::default(),
            prompt: None,
            confirm_delete: None,
            batch: None,
            state: state::ShellState::load(),
            thumbs: HashMap::new(),
            thumbs_pending: std::collections::HashSet::new(),
            thumbs_failed: std::collections::HashSet::new(),
            ai: None,
            find: None,
            sem_index: None,
            sem_indexing: false,
            palette: None,
            palette_commands: crate::palette::build_command_catalog(),
        }
    }

    fn on_resize(_model: &Self::Model, width: u32, height: u32) -> Option<Self::Msg> {
        Some(Msg::Resized(width as f32, height as f32))
    }

    fn on_key(_model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        // Command palette abierto: el módulo se lleva todo el teclado (igual
        // que un modal). Máxima prioridad.
        if let Some(state) = _model.palette.as_ref() {
            return command_palette::on_key(state, e).map(Msg::Palette);
        }
        // Panel de IA abierto: Esc lo cierra (es un overlay de resultado).
        if _model.ai.is_some() {
            if matches!(e.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::AiClose);
            }
            return None;
        }
        // Find recursivo abierto: captura todo el teclado (modal). Tab alterna
        // el modo (nombre ↔ contenido); Enter corre / abre; flechas navegan.
        if _model.find.is_some() {
            return match &e.key {
                Key::Named(NamedKey::Escape) => Some(Msg::FindClose),
                Key::Named(NamedKey::Enter) => Some(Msg::FindSubmit),
                Key::Named(NamedKey::Tab) => Some(Msg::FindToggleMode),
                Key::Named(NamedKey::Backspace) => Some(Msg::FindBackspace),
                Key::Named(NamedKey::ArrowDown) => Some(Msg::FindNav(1)),
                Key::Named(NamedKey::ArrowUp) => Some(Msg::FindNav(-1)),
                Key::Named(NamedKey::Space) => Some(Msg::FindInput(" ".to_string())),
                Key::Character(c) => Some(Msg::FindInput(c.to_string())),
                _ => None,
            };
        }
        // Ctrl+Shift+P (canónico, igual que VS Code/nada) o Ctrl+P abren el
        // palette. Va antes que cualquier captura modal-light de abajo.
        if command_palette::open_shortcut(e)
            || (e.modifiers.ctrl
                && !e.modifiers.shift
                && matches!(&e.key, Key::Character(c) if c.eq_ignore_ascii_case("p")))
        {
            return Some(Msg::Palette(PaletteMsg::Open));
        }
        // Ctrl+F abre el find recursivo (la `/` sigue siendo el filtro vivo
        // de la carpeta actual — son dos cosas: filtro local vs. find de árbol).
        if e.modifiers.ctrl
            && matches!(&e.key, Key::Character(c) if c.eq_ignore_ascii_case("f"))
        {
            return Some(Msg::FindOpen);
        }
        // Ctrl+I: pregunta a la IA sobre la selección (archivo/carpeta/marca).
        if e.modifiers.ctrl
            && matches!(&e.key, Key::Character(c) if c.eq_ignore_ascii_case("i"))
        {
            return Some(Msg::AiAsk);
        }
        // Prompt de nombre (nueva carpeta/archivo, renombrar): captura todo el
        // teclado. Máxima prioridad — es un modal.
        if _model.prompt.is_some() {
            return match &e.key {
                Key::Named(NamedKey::Escape) => Some(Msg::PromptCancel),
                Key::Named(NamedKey::Enter) => Some(Msg::PromptSubmit),
                Key::Named(NamedKey::Backspace) => Some(Msg::PromptBackspace),
                Key::Named(NamedKey::Space) => Some(Msg::PromptInput(" ".to_string())),
                Key::Character(c) => Some(Msg::PromptInput(c.to_string())),
                _ => None,
            };
        }
        // Renombrado por lote: el teclado edita el patrón. Enter aplica.
        if _model.batch.is_some() {
            return match &e.key {
                Key::Named(NamedKey::Escape) => Some(Msg::BatchCancel),
                Key::Named(NamedKey::Enter) => Some(Msg::BatchApply),
                Key::Named(NamedKey::Backspace) => Some(Msg::BatchPatternBackspace),
                Key::Named(NamedKey::Space) => Some(Msg::BatchPatternInput(" ".to_string())),
                Key::Character(c) => Some(Msg::BatchPatternInput(c.to_string())),
                _ => None,
            };
        }
        // Diálogo de confirmación de borrado: Enter/y confirma, Esc/n cancela.
        if _model.confirm_delete.is_some() {
            return match &e.key {
                Key::Named(NamedKey::Enter) => Some(Msg::ConfirmDelete),
                Key::Character(c) if c == "y" => Some(Msg::ConfirmDelete),
                Key::Named(NamedKey::Escape) => Some(Msg::CancelConfirm),
                Key::Character(c) if c == "n" => Some(Msg::CancelConfirm),
                _ => None,
            };
        }
        // Menú principal abierto: las flechas navegan, Enter ejecuta, Esc
        // cierra. Tiene prioridad sobre la navegación del explorer.
        if let Some(mi) = _model.menu_open {
            let n = app_menu(_model).menus.len().max(1);
            return match &e.key {
                Key::Named(NamedKey::Escape) => Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowLeft) => Some(Msg::MenuOpen(Some((mi + n - 1) % n))),
                Key::Named(NamedKey::ArrowRight) => Some(Msg::MenuOpen(Some((mi + 1) % n))),
                Key::Named(NamedKey::ArrowDown) => Some(Msg::MenuNav(1)),
                Key::Named(NamedKey::ArrowUp) => Some(Msg::MenuNav(-1)),
                Key::Named(NamedKey::Enter) => Some(Msg::MenuActivate),
                _ => None,
            };
        }
        // Editor de texto del canvas abierto: el teclado es del editor.
        // Ctrl+S guarda; Esc sin selección/multicursor cierra; el resto va
        // al buffer (incluidos Ctrl+C/X/V/Z/Y, que resuelve el editor).
        if let Some(CanvasApp::Texto { editor, .. }) = &_model.canvas {
            if e.modifiers.ctrl {
                if let Key::Character(c) = &e.key {
                    if c == "s" {
                        return Some(Msg::CanvasSave);
                    }
                }
            }
            if matches!(e.key, Key::Named(NamedKey::Escape))
                && !editor.has_selection()
                && editor.extra_cursors.is_empty()
            {
                return Some(Msg::CanvasClose);
            }
            return Some(Msg::CanvasEditKey(e.clone()));
        }
        // Editor de imágenes del canvas: Ctrl+Z/Y/S y radio del pincel.
        if matches!(_model.canvas, Some(CanvasApp::Imagen(_))) {
            if e.modifiers.ctrl {
                if let Key::Character(c) = &e.key {
                    match c.as_str() {
                        "z" => return Some(Msg::CanvasTullpu(tullpu::Msg::Undo)),
                        "y" => return Some(Msg::CanvasTullpu(tullpu::Msg::Redo)),
                        "s" => return Some(Msg::CanvasTullpu(tullpu::Msg::Guardar)),
                        _ => {}
                    }
                }
            }
            if let Key::Character(c) = &e.key {
                match c.as_str() {
                    "[" => return Some(Msg::CanvasTullpu(tullpu::Msg::BumpRadio(-2))),
                    "]" => return Some(Msg::CanvasTullpu(tullpu::Msg::BumpRadio(2))),
                    _ => {}
                }
            }
        }
        // Canvas con imagen/media: Esc o ⌫ cierran y vuelven a la carpeta.
        if _model.canvas.is_some() {
            if matches!(
                e.key,
                Key::Named(NamedKey::Escape) | Key::Named(NamedKey::Backspace)
            ) {
                return Some(Msg::CanvasClose);
            }
        }
        // Modo búsqueda del mapa: captura todo el teclado para la consulta.
        if matches!(_model.preview, PreviewPane::Map(_)) && _model.map_view.searching {
            return match &e.key {
                Key::Named(NamedKey::Escape) => Some(Msg::MapSearchCancel),
                Key::Named(NamedKey::Enter) => Some(Msg::MapSearchSubmit),
                Key::Named(NamedKey::Backspace) => Some(Msg::MapSearchBackspace),
                Key::Named(NamedKey::Space) => Some(Msg::MapSearchInput(" ".to_string())),
                Key::Character(c) => Some(Msg::MapSearchInput(c.to_string())),
                _ => None,
            };
        }
        // Modo filtro vivo: captura el teclado para el filtro por nombre.
        if _model.nav_filtering {
            return match &e.key {
                Key::Named(NamedKey::Escape) | Key::Named(NamedKey::Enter) => Some(Msg::NavFilterEnd),
                Key::Named(NamedKey::Backspace) => Some(Msg::NavFilterBackspace),
                Key::Named(NamedKey::Space) => Some(Msg::NavFilterInput(" ".to_string())),
                Key::Character(c) => Some(Msg::NavFilterInput(c.to_string())),
                _ => None,
            };
        }
        match &e.key {
            Key::Named(NamedKey::ArrowUp) => Some(Msg::Up),
            Key::Named(NamedKey::ArrowDown) => Some(Msg::Down),
            Key::Named(NamedKey::Enter) => Some(Msg::OpenSelected),
            Key::Named(NamedKey::Backspace) => Some(Msg::Parent),
            // Esc con el visor abierto vuelve a la vista de carpeta (el
            // handler de Parent cierra el visor antes de subir de dir).
            Key::Named(NamedKey::Escape) if _model.viewer_open => Some(Msg::Parent),
            // → expande inline la carpeta seleccionada; ← colapsa (o salta
            // al padre si ya está colapsada) — lista/detalle.
            Key::Named(NamedKey::ArrowRight) => Some(Msg::ExpandSelected),
            Key::Named(NamedKey::ArrowLeft) => Some(Msg::CollapseSelected),
            Key::Named(NamedKey::Space) => Some(Msg::TogglePlay),
            // `v` alterna lista/detalle, `/` filtra (salvo que un mapa quiera
            // `/` para su propia búsqueda, que tiene su arm más abajo).
            Key::Character(c) if c == "v" => Some(Msg::NavToggleView),
            Key::Character(c) if c == "/" && !matches!(_model.preview, PreviewPane::Map(_)) => {
                Some(Msg::NavFilterStart)
            }
            // `d` alterna panel doble; Tab cambia el foco entre los dos.
            Key::Character(c) if c == "d" => Some(Msg::ToggleDual),
            Key::Named(NamedKey::Tab) => Some(Msg::SwitchFocus),
            // Selección (parity dOpus): Ctrl+A marca todo, `*` invierte la
            // marca (como el numpad-* de Directory Opus).
            Key::Character(c) if c.eq_ignore_ascii_case("a") && e.modifiers.ctrl => {
                Some(Msg::SelectAll)
            }
            Key::Character(c) if c == "*" => Some(Msg::InvertSelection),
            // ---- Fase 4.3: operaciones de archivo (sólo sobre POSIX). ----
            // Marcar/desmarcar (selección múltiple) bajo el cursor. Útil tanto
            // sobre POSIX (operaciones de archivo) como dentro de un grafo de
            // Mónadas (submonadizar/fusionar la selección).
            Key::Named(NamedKey::Insert)
                if _model.can_edit() || _model.cur().monad_graph().is_some() =>
            {
                Some(Msg::ToggleMark)
            }
            // F7 nueva carpeta · F2 renombrar · Delete borrar.
            Key::Named(NamedKey::F7) if _model.can_edit() => Some(Msg::NewDirPrompt),
            Key::Named(NamedKey::F2) if _model.can_edit() => Some(Msg::RenamePrompt),
            Key::Named(NamedKey::Delete) if _model.can_edit() => Some(Msg::DeleteSelection),
            // F5 copiar / F6 mover al otro panel (sólo en dual).
            Key::Named(NamedKey::F5)
                if _model.dual && (_model.can_edit() || _model.activo_es_dispositivo()) =>
            {
                Some(Msg::CopyToOther)
            }
            Key::Named(NamedKey::F6) if _model.can_edit() && _model.dual => Some(Msg::MoveToOther),
            // Puntos de entrada del front universal: montar el directorio
            // objetivo (el subdir seleccionado, o el cwd) como otra `Source`.
            // Sólo desde POSIX — dentro de una fuente montada no aplican.
            Key::Character(c) if c == "m" => Some(Msg::MountNouser),
            Key::Character(c) if c == "g" => Some(Msg::MountMinga),
            // Sobre un mapa: `f` reencuadra, `b` alterna el mapa-base.
            Key::Character(c) if c == "f" && matches!(_model.preview, PreviewPane::Map(_)) => {
                Some(Msg::MapReset)
            }
            Key::Character(c) if c == "b" && matches!(_model.preview, PreviewPane::Map(_)) => {
                Some(Msg::MapToggleBase)
            }
            Key::Character(c) if c == "c" && matches!(_model.preview, PreviewPane::Map(_)) => {
                Some(Msg::MapCycleColor)
            }
            Key::Character(c) if c == "/" && matches!(_model.preview, PreviewPane::Map(_)) => {
                Some(Msg::MapSearchStart)
            }
            Key::Character(c) if c == "r" && matches!(_model.preview, PreviewPane::Map(_)) => {
                Some(Msg::MapRouteToggle)
            }
            _ => None,
        }
    }

    fn on_wheel(
        model: &Self::Model,
        delta: WheelDelta,
        cursor: (f32, f32),
        _mods: Modifiers,
    ) -> Option<Self::Msg> {
        // La rueda sobre el sidebar del árbol va SIEMPRE al árbol — ruteo por
        // región: el hit-test del `on_scroll` local se pierde entre updates
        // rápidos (el cache de render se invalida en cada update), y el
        // sobrante caía acá moviendo el canvas.
        if cursor.0 < model.tree_w {
            return Some(Msg::TreeScroll(delta.y));
        }
        // Con una app de canvas abierta y el toolbox en modo **lista**, la
        // rueda pasa al archivo siguiente/anterior de la carpeta (visor de
        // fotos). En modo zoom la rueda sigue siendo de la app (más abajo).
        if model.canvas.is_some() && model.wheel_mode == WheelMode::Lista {
            if delta.y > 0.3 {
                return Some(Msg::CanvasNav(1));
            }
            if delta.y < -0.3 {
                return Some(Msg::CanvasNav(-1));
            }
            return None;
        }
        // Si la rueda cae sobre el panel del mapa, hace zoom de la cámara en
        // vez de scrollear la lista (gateo por el rect que el canvas registra).
        if matches!(model.preview, PreviewPane::Map(_)) && model.map_view.contains(cursor.0, cursor.1)
        {
            return Some(Msg::MapZoom(delta.y, cursor.0, cursor.1));
        }
        // El delta del touchpad se acumula en `FileExplorerState`; acá
        // sólo aproximamos los pasos para evitar un round-trip por
        // sub-fila. El update llamará a `apply_wheel(delta.y)` para que
        // el acumulador real viva en el explorer, no en el shell.
        let steps = delta.y.trunc() as i32;
        Some(Msg::Scroll(steps))
    }

    fn update(model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        crate::update::shell_update(model, msg, handle)
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        crate::view::shell_view(model)
    }

    fn view_overlay(model: &Self::Model) -> Option<View<Self::Msg>> {
        crate::view::shell_view_overlay(model)
    }
}
