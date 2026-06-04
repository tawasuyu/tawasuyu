//! Vistas: las tres columnas (documentos · editor/diff · panel LLM), la
//! barra de status, el overlay de find, y las secciones de hijas/historial.

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_theme::Theme;
use llimphi_ui::{DragPhase, View};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use llimphi_widget_context_menu::{context_menu_view_ex, ContextMenuExtras};
use llimphi_widget_edit_menu::{self as editmenu, EditFlags};
use llimphi_widget_menubar::{
    menubar_overlay_animated, menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H,
};
use llimphi_widget_text_editor::{EditorPalette as TEPalette, Language};
use llimphi_widget_text_input::{text_input_view, TextInputPalette};
use pluma_align::CartaHebras;
use pluma_cuerpo::Cuerpo;
use pluma_editor_llimphi::cuerpo_ide::cuerpo_ide_view;
use pluma_editor_llimphi::multilienzo::{
    multilienzo_view, IndiceAtoms, MultilienzoConfig, PaletaHebras,
};
use pluma_editor_llimphi::Palette as MultPalette;
use pluma_transform::Transformacion;
use uuid::Uuid;

use crate::model::{Model, Msg, BACKENDS, METRICS, VISIBLE_LINES};
use crate::update::{contar_stale_del_activo, menu_principal};
use crate::util::{etiqueta_backend, etiqueta_intencion, etiqueta_tipo, recortar};

/// Tamaño de ventana del init — usado como viewport para clampear los
/// dropdowns del menú (la app no trackea el tamaño real hoy).
const VIEWPORT: (f32, f32) = (1600.0, 900.0);

/// Arma el `MenuBarSpec` compartido entre `menubar_view` (barra) y
/// `menubar_overlay` (dropdown). El `menu` se construye afuera y se
/// presta por referencia para no clonarlo dos veces.
fn menubar_spec<'a>(
    menu: &'a app_bus::AppMenu,
    model: &Model,
    theme: &'a Theme,
) -> MenuBarSpec<'a, Msg> {
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: VIEWPORT,
        height: MENU_H,
        on_open: std::sync::Arc::new(Msg::MenuOpen),
        on_command: std::sync::Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

pub(crate) fn vista(model: &Model) -> View<Msg> {
    let theme = Theme::dark();
    let editor_palette = TEPalette::default();
    let splitter_palette = SplitterPalette::from_theme(&theme);

    let menu = menu_principal(model);
    let menubar = menubar_view(&menubar_spec(&menu, model, &theme));
    let status = barra_status(model, &theme);
    let panel_centro = panel_editor(model, &editor_palette);

    // En modo delegado (sidebar prestado a pata) las columnas laterales se pueden
    // colapsar desde el rail de pata → editor a pantalla completa ("puro canvas").
    // Sin delegar, ambas van siempre (comportamiento original).
    let show_izq = !model.delegated || model.side_izq_visible;
    let show_der = !model.delegated || model.side_der_visible;

    // Splitter anidado: izq | (centro | der). Cada lado oculto se saca del árbol
    // (y con él su splitter), no sólo se esconde.
    let centro_der = if show_der {
        splitter_two(
            Direction::Row,
            panel_centro,
            PaneSize::Flex,
            panel_llm(model, &theme),
            PaneSize::Fixed(model.side_der_w),
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::ResizeDer(dx)),
                DragPhase::End => None,
            },
            &splitter_palette,
        )
    } else {
        panel_centro
    };
    let body = if show_izq {
        splitter_two(
            Direction::Row,
            panel_documentos(model, &theme),
            PaneSize::Fixed(model.side_izq_w),
            centro_der,
            PaneSize::Flex,
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::ResizeIzq(dx)),
                DragPhase::End => None,
            },
            &splitter_palette,
        )
    } else {
        centro_der
    };

    // El right-click se engancha en la raíz (origen 0,0 → las coords
    // locales que llegan al handler ya son de ventana) y abre el menú de
    // edición sobre el cuerpo_ide. La barra de menú va de primer hijo.
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .on_right_click_at(|x, y, _w, _h| Some(Msg::EditMenuOpen(x, y)))
    .children(vec![menubar, status, body])
}

/// Overlay flotante: el menú de edición contextual tiene prioridad; si no
/// está abierto, cae al dropdown del menú principal. La app no tenía otros
/// popups flotantes (find es inline), así que estos dos son todo.
pub(crate) fn vista_overlay(model: &Model) -> Option<View<Msg>> {
    let theme = Theme::dark();
    if let Some((x, y)) = model.edit_menu {
        let flags = EditFlags::from_editor(&model.ide.state, false);
        let mut spec = editmenu::edit_context_menu(
            (x, y),
            VIEWPORT,
            &theme,
            flags,
            Msg::EditMenuAction,
            Msg::CloseMenus,
        );
        spec.active = model.edit_active;
        return Some(context_menu_view_ex(
            spec,
            ContextMenuExtras {
                appear: model.edit_anim.value(),
                ..Default::default()
            },
        ));
    }
    let menu = menu_principal(model);
    menubar_overlay_animated(
        &menubar_spec(&menu, model, &theme),
        model.menu_active,
        model.menu_anim.value(),
    )
}

fn barra_status(model: &Model, theme: &Theme) -> View<Msg> {
    let nombre = model
        .activo
        .and_then(|id| model.cuerpos.iter().find(|c| c.id == id))
        .map(|c| c.metadatos.nombre_legible.clone())
        .unwrap_or_else(|| "(sin doc)".to_string());
    let zona = model.ide.zona_del_caret();
    let n_zonas = model.ide.n_zonas();
    let backend = etiqueta_backend(BACKENDS[model.backend_idx]);
    let estado = if model.en_curso {
        "⏳"
    } else if model.ultimo_error.is_some() {
        "⚠"
    } else {
        "○"
    };
    let texto = format!(
        "pluma · {nombre} · zona {zona}/{n_zonas} · backend {backend} · {estado} {}",
        model.ultimo_status
    );
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(30.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(texto, 12.0, theme.fg_text, Alignment::Start)
}

fn panel_documentos(model: &Model, theme: &Theme) -> View<Msg> {
    let palette_btn = ButtonPalette::from_theme(theme);
    let palette_list = ListPalette::from_theme(theme);

    // Originales primero, luego derivadas — el orden en la lista es
    // estable porque clonamos `model.cuerpos` y particionamos.
    let mut originales: Vec<&Cuerpo> = Vec::new();
    let mut derivadas: Vec<&Cuerpo> = Vec::new();
    for c in &model.cuerpos {
        if c.metadatos.intencion.es_derivada() {
            derivadas.push(c);
        } else {
            originales.push(c);
        }
    }

    let mut rows: Vec<ListRow<Msg>> = Vec::new();
    for c in originales.iter().chain(derivadas.iter()) {
        let prefijo = if c.metadatos.intencion.es_derivada() {
            "  ↳ "
        } else {
            "■ "
        };
        let label = format!(
            "{prefijo}{} · {}",
            c.metadatos.nombre_legible, c.branch_id
        );
        rows.push(ListRow {
            label,
            selected: model.activo == Some(c.id),
            on_click: Msg::AbrirDoc(c.id),
        });
    }

    let n = rows.len();
    let lista = list_view(ListSpec {
        rows,
        total: n,
        caption: Some(format!("{n} documentos")),
        truncated_hint: None,
        row_height: 22.0,
        palette: palette_list,
    });

    let boton_nuevo = button_view::<Msg>("＋  nuevo doc  (Ctrl+N)", &palette_btn, Msg::NuevoDoc);
    let boton_guardar = button_view::<Msg>("💾  guardar  (Ctrl+S)", &palette_btn, Msg::Guardar);

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .text_aligned("DOCUMENTOS".to_string(), 10.0, theme.fg_muted, Alignment::Start);

    let archivo = seccion_archivo(model, theme);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .clip(true)
    .children(vec![header, boton_nuevo, boton_guardar, archivo, lista])
}

fn seccion_archivo(model: &Model, theme: &Theme) -> View<Msg> {
    let palette_btn = ButtonPalette::from_theme(theme);
    let palette_input = TextInputPalette::from_theme(theme);

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(18.0_f32),
        },
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        "ARCHIVO".to_string(),
        10.0,
        theme.fg_muted,
        Alignment::Start,
    );

    let input = text_input_view::<Msg>(
        &model.path_input,
        "ruta .md o .docx (Esc para salir)",
        model.path_focused,
        &palette_input,
        Msg::FocusPath,
    );

    let fila_botones = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        button_view::<Msg>("📂 abrir", &palette_btn, Msg::AbrirArchivo),
        button_view::<Msg>("⬆ exportar (md/docx)", &palette_btn, Msg::ExportarMd),
    ]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: length(82.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        ..Default::default()
    })
    .children(vec![header, input, fila_botones])
}

fn panel_editor(model: &Model, palette_editor: &TEPalette) -> View<Msg> {
    let cuerpo_central: View<Msg> = if model.diff_visible {
        vista_diff(model, palette_editor)
    } else {
        cuerpo_ide_view::<Msg>(
            &model.ide,
            palette_editor,
            METRICS,
            VISIBLE_LINES,
            Language::Plain,
            |ev| Some(Msg::EditorPointer(ev)),
        )
    };

    let mut hijos: Vec<View<Msg>> = Vec::new();
    if model.find_visible {
        hijos.push(barra_find(model));
    }
    hijos.push(cuerpo_central);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .fill(palette_editor.bg)
    .clip(true)
    .children(hijos)
}

fn vista_diff(model: &Model, palette_editor: &TEPalette) -> View<Msg> {
    // Resolver activo + madre. Si activo no es derivado o la madre no
    // se encuentra, mostramos un cartel y volvemos a `cuerpo_ide_view`.
    let theme = Theme::dark();
    let activo_id = match model.activo {
        Some(id) => id,
        None => return cartel_diff("sin doc activo", palette_editor),
    };
    let activo = match model.cuerpos.iter().find(|c| c.id == activo_id) {
        Some(c) => c,
        None => return cartel_diff("activo no encontrado", palette_editor),
    };
    let madre_id = match activo.metadatos.derivado_de {
        Some(id) => id,
        None => {
            // Activo es Original — fallback al editor normal con cartel.
            return View::new(Style {
                flex_direction: FlexDirection::Column,
                size: Size {
                    width: percent(1.0_f32),
                    height: percent(1.0_f32),
                },
                gap: Size {
                    width: length(0.0_f32),
                    height: length(4.0_f32),
                },
                ..Default::default()
            })
            .children(vec![
                cartel_diff(
                    "este cuerpo es Original — no tiene madre con que diffear (Ctrl+D para cerrar)",
                    palette_editor,
                ),
                cuerpo_ide_view::<Msg>(
                    &model.ide,
                    palette_editor,
                    METRICS,
                    VISIBLE_LINES,
                    Language::Plain,
                    |ev| Some(Msg::EditorPointer(ev)),
                ),
            ]);
        }
    };
    let madre = match model.cuerpos.iter().find(|c| c.id == madre_id) {
        Some(c) => c,
        None => return cartel_diff(
            "madre referenciada no está en el sled — ¿borrada?",
            palette_editor,
        ),
    };

    // Buscar la carta de hebras entre estos dos. `pluma_align::CartaHebras`
    // anota su par; consideramos cualquier orden.
    let carta = model.cartas.iter().find(|c| {
        (c.cuerpo_a == Some(madre.id) && c.cuerpo_b == Some(activo.id))
            || (c.cuerpo_a == Some(activo.id) && c.cuerpo_b == Some(madre.id))
    });

    let cuerpos_ref: Vec<&Cuerpo> = vec![madre, activo];
    let cartas_ref: Vec<Option<&CartaHebras>> = vec![carta];
    let atoms_idx: IndiceAtoms = model.atoms.iter().map(|(k, v)| (*k, v)).collect();
    let cfg = MultilienzoConfig::default();
    let paleta_hebras = PaletaHebras::default();
    let palette_mult = MultPalette::from_theme(&theme);

    let mult = multilienzo_view::<Msg>(
        &cuerpos_ref,
        &atoms_idx,
        &cartas_ref,
        &cfg,
        &paleta_hebras,
        &palette_mult,
    );

    let header_text = format!(
        "DIFF · madre «{}» ↔ hija «{}» ({})",
        madre.metadatos.nombre_legible,
        activo.metadatos.nombre_legible,
        if carta.is_some() {
            "con hebras"
        } else {
            "sin carta — hebras no disponibles"
        },
    );
    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(header_text, 11.0, theme.fg_muted, Alignment::Start);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        ..Default::default()
    })
    .children(vec![header, mult])
}

fn cartel_diff(texto: &str, palette_editor: &TEPalette) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(40.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        texto.to_string(),
        12.0,
        palette_editor.fg_line_number,
        Alignment::Start,
    )
}

fn barra_find(model: &Model) -> View<Msg> {
    let theme = Theme::dark();
    let palette_input = TextInputPalette::from_theme(&theme);
    let palette_btn = ButtonPalette::from_theme(&theme);

    let input = text_input_view::<Msg>(
        &model.find_input,
        "buscar (Enter siguiente · Shift+Enter previo · Esc cerrar)",
        true, // find_visible implica que tiene foco
        &palette_input,
        Msg::FindToggle, // click en el input no cambia foco — siempre vivo
    );

    let total = model.find_matches.len();
    let pos = if total == 0 {
        0
    } else {
        model.find_idx + 1
    };
    let counter = View::new(Style {
        size: Size {
            width: length(80.0_f32),
            height: length(34.0_f32),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(8.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        format!("{pos}/{total}"),
        12.0,
        theme.fg_muted,
        Alignment::Center,
    );

    let prev = button_view::<Msg>("◀", &palette_btn, Msg::FindAnterior);
    let next = button_view::<Msg>("▶", &palette_btn, Msg::FindSiguiente);
    let cerrar = button_view::<Msg>("✕", &palette_btn, Msg::FindClose);

    let input_wrap = View::new(Style {
        flex_grow: 1.0,
        flex_shrink: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0_f32),
        },
        ..Default::default()
    })
    .children(vec![input]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(40.0_f32),
        },
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![input_wrap, counter, prev, next, cerrar])
}

fn panel_llm(model: &Model, theme: &Theme) -> View<Msg> {
    let palette_btn_activo = ButtonPalette::from_theme(theme);
    let palette_btn_off = ButtonPalette {
        bg: Color::from_rgba8(60, 60, 60, 255),
        bg_hover: Color::from_rgba8(60, 60, 60, 255),
        fg: Color::from_rgba8(140, 140, 140, 255),
        radius: palette_btn_activo.radius,
    };
    let pal = if model.en_curso {
        &palette_btn_off
    } else {
        &palette_btn_activo
    };
    let pal_backend = &palette_btn_activo;

    let etiqueta_back = format!(
        "🔀  backend: {}",
        etiqueta_backend(BACKENDS[model.backend_idx])
    );
    let cycler = button_view::<Msg>(&etiqueta_back, pal_backend, Msg::CicloBackend);

    let etiqueta_diff = if model.diff_visible {
        "↔  diff: ON  (Ctrl+D)"
    } else {
        "↔  diff: off  (Ctrl+D)"
    };
    let diff_btn = button_view::<Msg>(etiqueta_diff, pal_backend, Msg::DiffToggle);

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .text_aligned("LLM".to_string(), 10.0, theme.fg_muted, Alignment::Start);

    let mk = |label: &str, m: Msg| button_view::<Msg>(label, pal, m);
    let botones: Vec<View<Msg>> = vec![
        mk("→  traducir qu", Msg::PedirTraducir("qu".into())),
        mk("→  traducir en", Msg::PedirTraducir("en".into())),
        mk("✎  tono formal", Msg::PedirTono("formal".into())),
        mk("✂  resumir 30p", Msg::PedirResumir(Some(30))),
    ];

    let n_stale = contar_stale_del_activo(model);
    let label_regen = if n_stale > 0 {
        format!("⟳  regenerar stale ({n_stale})")
    } else {
        "⟳  regenerar stale (0)".to_string()
    };
    let tocar_btn = button_view::<Msg>("⏰  tocar madre", pal, Msg::TocarMadre);
    let regen_btn = button_view::<Msg>(&label_regen, pal, Msg::RegenerarStale);

    // Lista de hijas del cuerpo activo — para abrirlas con click.
    let hijas_seccion = seccion_hijas(model, theme);

    let mut hijos: Vec<View<Msg>> = Vec::new();
    hijos.push(header);
    hijos.push(cycler);
    hijos.push(diff_btn);
    hijos.extend(botones);
    hijos.push(tocar_btn);
    hijos.push(regen_btn);
    hijos.push(divider(theme));
    hijos.push(hijas_seccion);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .clip(true)
    .children(hijos)
}

fn seccion_hijas(model: &Model, theme: &Theme) -> View<Msg> {
    let palette_list = ListPalette::from_theme(theme);
    let activo = model.activo;

    let hijas: Vec<&Cuerpo> = model
        .cuerpos
        .iter()
        .filter(|c| {
            c.metadatos.intencion.es_derivada() && c.metadatos.derivado_de == activo
        })
        .collect();

    let mut rows: Vec<ListRow<Msg>> = Vec::new();
    for h in &hijas {
        let label = format!("• {} · {}", h.branch_id, etiqueta_intencion(&h.metadatos.intencion));
        rows.push(ListRow {
            label,
            selected: false,
            on_click: Msg::AbrirDoc(h.id),
        });
    }

    let n = rows.len();
    let lista = list_view(ListSpec {
        rows,
        total: n,
        caption: Some(format!("hijas: {n}")),
        truncated_hint: None,
        row_height: 20.0,
        palette: palette_list,
    });

    let historial = seccion_historial(model, theme);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        flex_grow: 1.0,
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .children(vec![lista, divider(theme), historial])
}

fn seccion_historial(model: &Model, theme: &Theme) -> View<Msg> {
    let palette_list = ListPalette::from_theme(theme);

    // Index para resolver Uuid → Cuerpo, cuerpo.metadatos.nombre_legible.
    let cuerpo_de = |id: Uuid| model.cuerpos.iter().find(|c| c.id == id);

    // Transformaciones del cuerpo activo: ya sea como madre o como hija.
    // Lo más útil al usuario suele ser "todo lo que pasó alrededor de
    // este doc" — así una hija de cuya madre vengo, lo veo.
    let activo = model.activo;
    let mut filtradas: Vec<&Transformacion> = model
        .transformaciones
        .iter()
        .filter(|t| match activo {
            Some(id) => t.madre == id || t.hija == id,
            None => true,
        })
        .collect();
    // Más recientes arriba.
    filtradas.sort_by(|a, b| b.creada_en.cmp(&a.creada_en));

    let mut rows: Vec<ListRow<Msg>> = Vec::new();
    for t in &filtradas {
        let madre = cuerpo_de(t.madre)
            .map(|c| c.metadatos.nombre_legible.as_str())
            .unwrap_or("?");
        let hija = cuerpo_de(t.hija)
            .map(|c| c.metadatos.nombre_legible.as_str())
            .unwrap_or("?");
        let tipo = etiqueta_tipo(&t.tipo);
        // Truncar nombres largos para que la fila no se rompa visual.
        let label = format!(
            "{}  →  {}  ·  {}",
            recortar(madre, 18),
            recortar(hija, 18),
            tipo,
        );
        rows.push(ListRow {
            label,
            selected: false,
            on_click: Msg::AbrirDoc(t.hija),
        });
    }

    let n = rows.len();
    let lista = list_view(ListSpec {
        rows,
        total: n,
        caption: Some(if activo.is_some() {
            format!("historial activo: {n}")
        } else {
            format!("historial: {n}")
        }),
        truncated_hint: None,
        row_height: 20.0,
        palette: palette_list,
    });

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        ..Default::default()
    })
    .children(vec![lista])
}

fn divider(theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.border)
}
