//! Chrome del navegador: la UI que rodea al contenido de la página. Menús
//! (menubar/app menu/edit menu + `FocusTarget`), feature de búsqueda en página
//! (`Matcher`, `count_matches`, `find_match_y*`, `find_bar`/`find_toggle`),
//! paneles laterales (bookmarks/historial/fuente) y la barra de pestañas
//! horizontal (`tabs_bar`). Extraído de `lib.rs` (regla #1). El header de
//! dirección renovado (theme-driven, con autocompletar) vive en `container.rs`
//! (`nav_header_bar`). Comparte todos los tipos del crate vía `use super::*`.
use super::*;
use rimay_localize::{t, t_args};

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
pub(crate) fn menubar_spec<'a>(menu: &'a app_bus::AppMenu, model: &Model) -> MenuBarSpec<'a, Msg> {
    let (w, h) = Puriy::initial_size();
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme: menu_theme(),
        viewport: (w as f32, h as f32),
        height: MENU_H,
        on_open: std::sync::Arc::new(Msg::MenuOpen),
        on_command: std::sync::Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// Tema fijo para la barra/menús — puriy no trackea un `Theme` en su
/// Model (su chrome usa colores claros hard-coded), así que sostenemos un
/// `Theme::dark()` fijo y compartido para los menús. `OnceLock` lo
/// inicializa una sola vez sin `unsafe`.
pub(crate) fn menu_theme() -> &'static Theme {
    static CELL: std::sync::OnceLock<Theme> = std::sync::OnceLock::new();
    CELL.get_or_init(Theme::dark)
}

/// Menú principal del navegador. Sólo expone comandos que mapean a
/// `Msg` reales ya existentes. El submenú Editar refleja en gris el
/// estado del campo de texto focuseado (find/filtro/input de página/
/// address bar).
pub(crate) fn app_menu(model: &Model) -> app_bus::AppMenu {
    use app_bus::{AppMenu, Menu, MenuItem};

    let focused = model.focused_text_input();
    let has_sel = focused.map(|(s, _)| s.editor().has_selection()).unwrap_or(false);
    let can_undo = focused.map(|(s, _)| s.editor().can_undo()).unwrap_or(false);
    let can_redo = focused.map(|(s, _)| s.editor().can_redo()).unwrap_or(false);
    let has_input = focused.is_some();

    let mut undo = MenuItem::new(t("undo"), "edit.undo").shortcut("Ctrl+Z");
    if !can_undo { undo = undo.disabled(); }
    let mut redo = MenuItem::new(t("redo"), "edit.redo").shortcut("Ctrl+Y");
    if !can_redo { redo = redo.disabled(); }
    let mut cut = MenuItem::new(t("cut"), "edit.cut").shortcut("Ctrl+X").separated();
    let mut copy = MenuItem::new(t("copy"), "edit.copy").shortcut("Ctrl+C");
    if !has_sel { cut = cut.disabled(); copy = copy.disabled(); }
    let mut paste = MenuItem::new(t("paste"), "edit.paste").shortcut("Ctrl+V");
    let mut sel_all =
        MenuItem::new(t("puriy-select-all"), "edit.selectall").shortcut("Ctrl+A").separated();
    if !has_input { paste = paste.disabled(); sel_all = sel_all.disabled(); }

    let tab = model.active();
    let can_back = tab.can_back();
    let can_fwd = tab.can_fwd();
    let mut back = MenuItem::new(t("puriy-back"), "nav.back").shortcut("Alt+←");
    if !can_back { back = back.disabled(); }
    let mut fwd = MenuItem::new(t("puriy-forward"), "nav.fwd").shortcut("Alt+→");
    if !can_fwd { fwd = fwd.disabled(); }

    AppMenu::new()
        .menu(
            Menu::new(t("puriy-menu-file"))
                .item(MenuItem::new(t("puriy-new-tab"), "file.newtab").shortcut("Ctrl+T"))
                .item(MenuItem::new(t("puriy-close-tab"), "file.close").shortcut("Ctrl+W").separated())
                .item(MenuItem::new(t("puriy-reload"), "file.reload").shortcut("F5"))
                .item(MenuItem::new(t("puriy-view-source"), "file.source").shortcut("Ctrl+U"))
                .item(MenuItem::new(t("puriy-add-bookmark"), "file.bookmark").shortcut("Ctrl+D")),
        )
        .menu(
            Menu::new(t("edit"))
                .item(undo)
                .item(redo)
                .item(cut)
                .item(copy)
                .item(paste)
                .item(sel_all),
        )
        .menu(
            Menu::new(t("puriy-menu-navigate"))
                .item(back)
                .item(fwd)
                .item(MenuItem::new(t("puriy-goto-addr"), "nav.addr").shortcut("Ctrl+L").separated())
                .item(MenuItem::new(t("puriy-find-in-page"), "nav.find").shortcut("Ctrl+F")),
        )
        .menu(
            Menu::new(t("puriy-menu-view"))
                .item(MenuItem::new(t("puriy-zoom-in"), "view.zoomin").shortcut("Ctrl++"))
                .item(MenuItem::new(t("puriy-zoom-out"), "view.zoomout").shortcut("Ctrl+-"))
                .item(MenuItem::new(t("puriy-zoom-reset"), "view.zoomreset").shortcut("Ctrl+0").separated())
                .item(MenuItem::new(t("puriy-bookmarks"), "view.bookmarks").shortcut("Ctrl+B"))
                .item(MenuItem::new(t("puriy-history"), "view.history").shortcut("Ctrl+H")),
        )
        .menu(
            Menu::new(t("help"))
                .item(MenuItem::new(t("puriy-about"), "help.about")),
        )
}

/// Traduce el `command` del menú principal al `Msg` real existente y lo
/// despacha por el `update`. Cierra el menú antes de actuar.
pub(crate) fn handle_menu_command(mut model: Model, command: String, handle: &Handle<Msg>) -> Model {
    model.menu_open = None;
    let target = match command.as_str() {
        "file.newtab" => Some(Msg::NewTab),
        "file.close" => Some(Msg::CloseTab(model.active)),
        "file.reload" => Some(Msg::Reload),
        "file.source" => Some(Msg::ViewSource),
        "file.bookmark" => Some(Msg::Bookmark),
        "edit.undo" => Some(Msg::EditMenuAction(EditAction::Undo)),
        "edit.redo" => Some(Msg::EditMenuAction(EditAction::Redo)),
        "edit.cut" => Some(Msg::EditMenuAction(EditAction::Cut)),
        "edit.copy" => Some(Msg::EditMenuAction(EditAction::Copy)),
        "edit.paste" => Some(Msg::EditMenuAction(EditAction::Paste)),
        "edit.selectall" => Some(Msg::EditMenuAction(EditAction::SelectAll)),
        "nav.back" => Some(Msg::Back),
        "nav.fwd" => Some(Msg::Forward),
        "nav.addr" => Some(Msg::FocusAddr),
        "nav.find" => Some(Msg::FindOpen),
        "view.zoomin" => Some(Msg::ZoomIn),
        "view.zoomout" => Some(Msg::ZoomOut),
        "view.zoomreset" => Some(Msg::ZoomReset),
        "view.bookmarks" => Some(Msg::ToggleBookmarks),
        "view.history" => Some(Msg::ToggleHistory),
        // "help.about" no tiene acción real — no-op silencioso.
        _ => None,
    };
    match target {
        Some(msg) => Puriy::update(model, msg, handle),
        None => model,
    }
}

/// Identifica qué campo de texto tiene el foco, para resolver borrows
/// disjuntos sin `unsafe` en `apply_edit_menu_action` (el clipboard y el
/// input son campos distintos del `Model`).
pub(crate) enum FocusTarget {
    Find,
    PanelFilter,
    PageInput(usize),
    Addr,
}

impl Model {
    /// Determina el `FocusTarget` con la misma prioridad que
    /// `focused_text_input`, sin tomar un borrow del input — así el caller
    /// puede pedir luego `&mut clipboard` + `&mut input` por separado.
    fn focus_target(&self) -> Option<FocusTarget> {
        if self.find_active {
            return Some(FocusTarget::Find);
        }
        if self.panel.is_some() {
            return Some(FocusTarget::PanelFilter);
        }
        let t = self.active();
        if let Some(idx) = t.focused_input {
            if idx < t.inputs.len() {
                return Some(FocusTarget::PageInput(idx));
            }
        }
        if t.addr_focused {
            return Some(FocusTarget::Addr);
        }
        None
    }
}

/// Aplica una `EditAction` del menú de edición sobre el `EditorState` del
/// campo de texto focuseado. Resuelve `clipboard` e `input` como borrows
/// disjuntos del `Model` (sin `unsafe`). Cierra el menú de edición.
pub(crate) fn apply_edit_menu_action(model: &mut Model, action: EditAction) {
    model.edit_menu = None;
    let Some(target) = model.focus_target() else { return };
    let active = model.active;
    match target {
        FocusTarget::Find => {
            editmenu::apply(model.find_input.editor_mut(), action, &mut model.clipboard);
        }
        FocusTarget::PanelFilter => {
            editmenu::apply(model.panel_filter.editor_mut(), action, &mut model.clipboard);
        }
        FocusTarget::PageInput(idx) => {
            if let Some(input) = model.tabs[active].inputs.get_mut(idx) {
                editmenu::apply(input.editor_mut(), action, &mut model.clipboard);
            }
        }
        FocusTarget::Addr => {
            editmenu::apply(model.tabs[active].addr.editor_mut(), action, &mut model.clipboard);
        }
    }
}

/// Walk del box tree contando hojas de texto que matchean el `matcher`
/// (query + toggles case/whole-word). Matcher vacío → 0 matches.
pub(crate) fn count_matches(tree: Option<&BoxTree>, matcher: &Matcher) -> usize {
    let Some(t) = tree else { return 0 };
    if matcher.is_empty() {
        return 0;
    }
    let mut count = 0_usize;
    t.walk(|b| {
        if let Some(txt) = &b.text {
            if matcher.matches(txt) {
                count += 1;
            }
        }
    });
    count
}

/// Opciones de coincidencia de la find bar (Fase 7.31). Default = búsqueda
/// case-insensitive por substring (comportamiento clásico de browsers).
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct MatchOpts {
    /// Distingue mayúsculas/minúsculas.
    pub case_sensitive: bool,
    /// Sólo matchea palabras completas (delimitadas por bordes
    /// no-alfanuméricos, incluyendo inicio/fin de la hoja de texto).
    pub whole_word: bool,
}

/// Predicado de búsqueda compilado: la query ya viene normalizada
/// (lowercased si no es case-sensitive) para no pagar el cast por hoja.
/// Reúne el matching de count/highlight/scroll en un solo lugar para que
/// las tres vistas cuenten exactamente los mismos matches en el mismo
/// orden DFS.
pub(crate) struct Matcher {
    needle: String,
    case_sensitive: bool,
    whole_word: bool,
}

impl Matcher {
    pub(crate) fn new(query: &str, opts: MatchOpts) -> Self {
        let needle = if opts.case_sensitive {
            query.to_string()
        } else {
            query.to_lowercase()
        };
        Matcher {
            needle,
            case_sensitive: opts.case_sensitive,
            whole_word: opts.whole_word,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.needle.is_empty()
    }

    /// `true` si `text` contiene al menos una ocurrencia de la query bajo
    /// las opciones activas.
    pub(crate) fn matches(&self, text: &str) -> bool {
        if self.needle.is_empty() {
            return false;
        }
        if self.case_sensitive {
            self.find_in(text)
        } else {
            self.find_in(&text.to_lowercase())
        }
    }

    /// `hay` ya viene normalizado (lowercased si corresponde) — busca la
    /// `needle` con o sin restricción de palabra completa.
    pub(crate) fn find_in(&self, hay: &str) -> bool {
        if !self.whole_word {
            return hay.contains(&self.needle);
        }
        // Whole-word: cada ocurrencia debe estar delimitada por bordes de
        // palabra (inicio/fin del string o un char no alfanumérico).
        // Caminamos char-aware para no romper en UTF-8 multibyte.
        let nlen = self.needle.len();
        let mut start = 0_usize;
        while let Some(pos) = hay[start..].find(&self.needle) {
            let i = start + pos;
            let before_ok = hay[..i].chars().next_back().map_or(true, |c| !is_word_char(c));
            let after_ok = hay[i + nlen..].chars().next().map_or(true, |c| !is_word_char(c));
            if before_ok && after_ok {
                return true;
            }
            // Avanzar al siguiente boundary de char válido para no panicar
            // en el próximo `find` ni quedar estancados.
            start = i + 1;
            while start < hay.len() && !hay.is_char_boundary(start) {
                start += 1;
            }
        }
        false
    }
}

/// Un caracter cuenta como "de palabra" si es alfanumérico (cualquier
/// alfabeto Unicode) o `_`. Lo demás (espacios, puntuación, símbolos)
/// es un borde de palabra.
pub(crate) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Estima la y del N-ésimo (1-based) leaf de texto que matchea el
/// `matcher`, acumulando alturas igual que `BoxTree::find_y_of_match` del
/// engine pero con el predicado configurable de la find bar (Fase 7.31).
/// Se replica acá en vez de extender el engine para no tocar `boxes.rs` —
/// el costo es ~15 líneas y mantiene el scroll consistente con los
/// toggles case/whole-word.
pub(crate) fn find_match_y(tree: &BoxTree, matcher: &Matcher, nth_1based: usize) -> Option<f32> {
    if matcher.is_empty() || nth_1based == 0 {
        return None;
    }
    let mut acc = 0.0_f32;
    let mut seen = 0_usize;
    find_match_y_inner(&tree.root, matcher, nth_1based, &mut acc, &mut seen)
}

pub(crate) fn find_match_y_inner(
    b: &BoxNode,
    matcher: &Matcher,
    target_nth: usize,
    acc: &mut f32,
    seen: &mut usize,
) -> Option<f32> {
    if let Some(text) = &b.text {
        if matcher.matches(text) {
            *seen += 1;
            if *seen == target_nth {
                return Some(*acc);
            }
        }
        *acc += b.font_size * b.line_height.unwrap_or(1.2);
        return None;
    }
    *acc += b.margin.top + b.padding.top;
    for c in &b.children {
        if let Some(y) = find_match_y_inner(c, matcher, target_nth, acc, seen) {
            return Some(y);
        }
    }
    *acc += b.margin.bottom + b.padding.bottom;
    None
}

/// Chip-toggle de la find bar (`Aa` case-sensitive / `W` whole-word).
/// Activo = fondo azul; inactivo = gris apagado. Click → `msg`.
pub(crate) fn find_toggle(label: &str, active: bool, msg: Msg) -> View<Msg> {
    let (bg, fg) = if active {
        (Color::from_rgb8(86, 124, 196), Color::from_rgb8(245, 245, 255))
    } else {
        (Color::from_rgb8(70, 70, 84), Color::from_rgb8(165, 165, 180))
    };
    View::new(Style {
        size: Size { width: length(26.0_f32), height: length(22.0_f32) },
        margin: Rect {
            left: length(6.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .radius(3.0)
    .text_aligned(label, 11.0, fg, Alignment::Center)
    .on_click(msg)
}

/// Find bar — input + contador + toggles (Aa/W) + close. Sticky entre
/// header y viewport mientras `find_active`.
pub(crate) fn find_bar(
    input: &TextInputState,
    count: usize,
    current: usize,
    case_sensitive: bool,
    whole_word: bool,
) -> View<Msg> {
    let palette = TextInputPalette::default();
    // Siempre focado mientras está abierta — Ctrl+F fue la última acción
    // explícita del usuario, no tiene sentido que el input no acepte teclas.
    let ph = t("puriy-find-placeholder");
    let entry = text_input_view(input, &ph, true, &palette, Msg::FindOpen);

    let count_label = if input.text().is_empty() {
        t("puriy-find-hint")
    } else if count == 0 {
        t("puriy-find-none")
    } else if current > 0 && current <= count {
        t_args(
            "puriy-find-pos",
            &[("cur", current.to_string().into()), ("total", count.to_string().into())],
        )
    } else if count == 1 {
        t("puriy-find-one")
    } else {
        t_args("puriy-find-count", &[("n", count.to_string().into())])
    };

    let close = View::new(Style {
        size: Size { width: length(22.0_f32), height: length(22.0_f32) },
        margin: Rect {
            left: length(8.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgb8(80, 80, 95))
    .radius(3.0)
    .text_aligned("✕", 12.0, Color::from_rgb8(220, 220, 230), Alignment::Center)
    .on_click(Msg::FindClose);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(34.0_f32) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgb8(50, 50, 62))
    .children(vec![
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
            ..Default::default()
        })
        .children(vec![entry]),
        View::new(Style {
            size: Size { width: length(120.0_f32), height: length(20.0_f32) },
            margin: Rect {
                left: length(8.0_f32),
                right: length(0.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(count_label, 11.0, Color::from_rgb8(200, 200, 215), Alignment::Start),
        find_toggle("Aa", case_sensitive, Msg::FindToggleCase),
        find_toggle("W", whole_word, Msg::FindToggleWord),
        close,
    ])
}

/// Panel auxiliar que reemplaza el viewport con la lista de bookmarks o
/// el historial. Lee directamente del Profile vía `profile_handle()`; si
/// el chrome corre sin profile (modo efímero) muestra un mensaje. El
/// filtro substring (case-insensitive) se aplica al title y url de cada
/// item; vacío = sin filtro.
pub(crate) fn panel_view(
    kind: PanelKind,
    filter: &TextInputState,
    source: Option<&str>,
    zoom: f32,
) -> View<Msg> {
    // Source: render directo, sin items / filtro relevante.
    if kind == PanelKind::Source {
        return source_panel(source, zoom);
    }
    let (title, all_items) = match kind {
        PanelKind::Bookmarks => collect_bookmarks(),
        PanelKind::History => collect_history(),
        PanelKind::Source => unreachable!(),
    };
    let q = filter.text();
    let q_lc = q.to_lowercase();
    let items: Vec<PanelItem> = if q_lc.is_empty() {
        all_items
    } else {
        all_items
            .into_iter()
            .filter(|it| {
                it.title.to_lowercase().contains(&q_lc)
                    || it.url.to_lowercase().contains(&q_lc)
            })
            .collect()
    };
    let title = if q_lc.is_empty() {
        title
    } else {
        t_args(
            "puriy-panel-filtered",
            &[("title", title.into()), ("n", items.len().to_string().into())],
        )
    };

    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(38.0_f32) },
        padding: Rect {
            left: length(16.0_f32),
            right: length(12.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgb8(35, 35, 45))
    .children(vec![
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(title, 13.0, Color::from_rgb8(230, 230, 240), Alignment::Start),
        View::new(Style {
            size: Size { width: length(22.0_f32), height: length(22.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(Color::from_rgb8(80, 80, 95))
        .radius(3.0)
        .text_aligned("✕", 12.0, Color::from_rgb8(220, 220, 230), Alignment::Center)
        .on_click(Msg::ClosePanel),
    ]);

    let list: Vec<View<Msg>> = if items.is_empty() {
        let msg = match kind {
            PanelKind::Bookmarks => t("puriy-bookmarks-empty"),
            PanelKind::History => t("puriy-history-empty"),
            PanelKind::Source => unreachable!(),
        };
        vec![View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(48.0_f32) },
            padding: Rect {
                left: length(16.0_f32),
                right: length(16.0_f32),
                top: length(16.0_f32),
                bottom: length(16.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(msg, 12.0, Color::from_rgb8(140, 140, 150), Alignment::Start)]
    } else {
        items.into_iter().map(panel_item_row).collect()
    };

    let body = View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .fill(Color::WHITE)
    .clip(true)
    .children(list);

    let palette = TextInputPalette::default();
    let placeholder = match kind {
        PanelKind::Bookmarks => t("puriy-filter-bookmarks"),
        PanelKind::History => t("puriy-filter-history"),
        PanelKind::Source => unreachable!(),
    };
    let filter_input = text_input_view(filter, &placeholder, true, &palette, Msg::ClosePanel);
    let filter_row = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(32.0_f32) },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgb8(45, 45, 55))
    .children(vec![View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
        ..Default::default()
    })
    .children(vec![filter_input])]);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![header, filter_row, body])
}

/// Panel "Page Source" — muestra el HTML crudo de la pestaña activa.
/// Línea por línea, prefijada por número (1-based, 4 dígitos). Mono
/// tamaño (12px × zoom), color foreground gris claro sobre fondo
/// oscuro estilo terminal. Sin scroll por ahora — Llimphi clipea
/// vertical; el usuario ve las primeras líneas.
pub(crate) fn source_panel(source: Option<&str>, zoom: f32) -> View<Msg> {
    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(38.0_f32) },
        padding: Rect {
            left: length(16.0_f32),
            right: length(12.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgb8(35, 35, 45))
    .children(vec![
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            t("puriy-source-header"),
            13.0,
            Color::from_rgb8(230, 230, 240),
            Alignment::Start,
        ),
        View::new(Style {
            size: Size { width: length(22.0_f32), height: length(22.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(Color::from_rgb8(80, 80, 95))
        .radius(3.0)
        .text_aligned("✕", 12.0, Color::from_rgb8(220, 220, 230), Alignment::Center)
        .on_click(Msg::ClosePanel),
    ]);

    let lines: Vec<View<Msg>> = match source {
        Some(src) if !src.is_empty() => src
            .lines()
            .enumerate()
            .take(2000) // cap protección — sources gigantes no destruyen el frame
            .map(|(i, line)| source_line_view(i + 1, line, zoom))
            .collect(),
        Some(_) => vec![source_empty_row(&t("puriy-source-no-body"))],
        None => vec![source_empty_row(&t("puriy-source-not-loaded"))],
    };

    let body = View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .fill(Color::from_rgb8(24, 24, 30))
    .clip(true)
    .children(lines);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![header, body])
}

pub(crate) fn source_line_view(num: usize, text: &str, zoom: f32) -> View<Msg> {
    let line_h = 16.0_f32 * zoom;
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(line_h) },
        flex_direction: FlexDirection::Row,
        ..Default::default()
    })
    .children(vec![
        View::new(Style {
            size: Size { width: length(48.0_f32 * zoom), height: length(line_h) },
            margin: Rect {
                left: length(0.0_f32),
                right: length(8.0_f32 * zoom),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(
            format!("{num:>4}"),
            11.0 * zoom,
            Color::from_rgb8(110, 110, 130),
            Alignment::End,
        ),
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(line_h) },
            ..Default::default()
        })
        .text_aligned(
            text.to_string(),
            12.0 * zoom,
            Color::from_rgb8(220, 220, 230),
            Alignment::Start,
        ),
    ])
}

pub(crate) fn source_empty_row(msg: &str) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(40.0_f32) },
        padding: Rect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(12.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(msg.to_string(), 12.0, Color::from_rgb8(140, 140, 150), Alignment::Start)
}

/// Item de panel: title arriba, url abajo (más chico/gris), click→navega.
/// `removable` con Some(id) agrega un botón ✕ que dispara
/// `Msg::RemoveBookmark(id)`.
pub(crate) struct PanelItem {
    title: String,
    url: String,
    removable: Option<puriy_core::BookmarkId>,
}

pub(crate) fn panel_item_row(item: PanelItem) -> View<Msg> {
    let nav_msg = Msg::Navigate(item.url.clone());
    let title_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        ..Default::default()
    })
    .text_aligned(
        truncate(&item.title, 80),
        13.0,
        Color::from_rgb8(30, 30, 40),
        Alignment::Start,
    );
    let url_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(16.0_f32) },
        ..Default::default()
    })
    .text_aligned(
        truncate(&item.url, 100),
        10.0,
        Color::from_rgb8(110, 110, 130),
        Alignment::Start,
    );
    let mut col_children = vec![title_view, url_view];
    let text_col = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(44.0_f32) },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .on_click(nav_msg)
    .children(std::mem::take(&mut col_children));

    let mut row_children = vec![text_col];
    if let Some(id) = item.removable {
        row_children.push(
            View::new(Style {
                size: Size { width: length(24.0_f32), height: length(24.0_f32) },
                margin: Rect {
                    left: length(8.0_f32),
                    right: length(0.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .fill(Color::from_rgb8(220, 220, 230))
            .radius(3.0)
            .text_aligned("✕", 11.0, Color::from_rgb8(80, 80, 95), Alignment::Center)
            .on_click(Msg::RemoveBookmark(id)),
        );
    }

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(54.0_f32) },
        padding: Rect {
            left: length(16.0_f32),
            right: length(12.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(1.0_f32),
        },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::WHITE)
    .hover_fill(Color::from_rgb8(238, 238, 245))
    .children(row_children)
}

/// Lee los bookmarks del Profile (si está cableado) y los devuelve como
/// items de panel con botón de borrar.
pub(crate) fn collect_bookmarks() -> (String, Vec<PanelItem>) {
    let Some(handle) = profile_handle() else {
        return (t("puriy-panel-bookmarks-noprofile"), Vec::new());
    };
    let Ok(p) = handle.lock() else {
        return (t("puriy-panel-bookmarks-bare"), Vec::new());
    };
    let items: Vec<PanelItem> = p
        .bookmarks
        .items()
        .iter()
        .map(|b| PanelItem {
            title: if b.title.is_empty() { b.url.clone() } else { b.title.clone() },
            url: b.url.clone(),
            removable: Some(b.id),
        })
        .collect();
    let title = t_args("puriy-panel-bookmarks-count", &[("n", items.len().to_string().into())]);
    (title, items)
}

/// Lee el historial del Profile y lo devuelve descendente (más reciente
/// primero), sin botón de borrado individual por ahora.
pub(crate) fn collect_history() -> (String, Vec<PanelItem>) {
    let Some(handle) = profile_handle() else {
        return (t("puriy-panel-history-noprofile"), Vec::new());
    };
    let Ok(p) = handle.lock() else {
        return (t("puriy-panel-history-bare"), Vec::new());
    };
    let items: Vec<PanelItem> = p
        .history
        .entries()
        .iter()
        .rev()
        .map(|h| PanelItem {
            title: if h.title.is_empty() { h.url.clone() } else { h.title.clone() },
            url: h.url.clone(),
            removable: None,
        })
        .collect();
    let title = t_args("puriy-panel-history-count", &[("n", items.len().to_string().into())]);
    (title, items)
}

pub(crate) fn tabs_bar(model: &Model) -> View<Msg> {
    let mut kids: Vec<View<Msg>> = Vec::with_capacity(model.tabs.len() + 1);
    for (i, t) in model.tabs.iter().enumerate() {
        let active = i == model.active;
        let bg = if active { Color::from_rgb8(245, 245, 248) } else { Color::from_rgb8(40, 40, 50) };
        let fg = if active { Color::from_rgb8(20, 20, 24) } else { Color::from_rgb8(200, 200, 210) };
        let label = if t.title.is_empty() { t.url.as_str() } else { t.title.as_str() };
        let close = View::new(Style {
            size: Size { width: length(18.0_f32), height: length(18.0_f32) },
            margin: Rect {
                left: length(6.0_f32),
                right: length(2.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned("✕", 11.0, fg, Alignment::Center)
        .on_click(Msg::CloseTab(i));

        let tab_view = View::new(Style {
            size: Size { width: length(180.0_f32), height: percent(1.0_f32) },
            padding: Rect {
                left: length(10.0_f32),
                right: length(6.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            margin: Rect {
                left: length(0.0_f32),
                right: length(2.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            flex_direction: FlexDirection::Row,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(bg)
        .radius(3.0)
        .on_click(Msg::SelectTab(i))
        .children(vec![
            View::new(Style {
                size: Size { width: length(140.0_f32), height: length(18.0_f32) },
                ..Default::default()
            })
            .text_aligned(truncate(label, 22), 11.0, fg, Alignment::Start),
            close,
        ]);
        kids.push(tab_view);
    }
    kids.push(
        View::new(Style {
            size: Size { width: length(28.0_f32), height: percent(1.0_f32) },
            margin: Rect {
                left: length(4.0_f32),
                right: length(0.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned("+", 16.0, Color::from_rgb8(200, 200, 210), Alignment::Center)
        .on_click(Msg::NewTab),
    );

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(TABS_H) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(4.0_f32),
            bottom: length(0.0_f32),
        },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgb8(18, 18, 22))
    .children(kids)
}
