//! Vistas: rail de dientes (Archivo · Lienzos · Derivar · Modelo) + panel del
//! diente activo + centro multilienzo (N editores lado a lado con hebras de
//! color y scroll horizontal) + barra de status + overlay de find.

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Position, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_icons::{icon_view, Icon};
use llimphi_theme::Theme;
use llimphi_ui::{DragPhase, View};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_context_menu::{context_menu_view_ex, ContextMenuExtras};
use llimphi_widget_dock_rail::{
    dock_rail_view, dock_rail_view_side, DockRailItem, DockRailPalette, DockRailSide,
};
use llimphi_widget_edit_menu::{self as editmenu, EditFlags};
use llimphi_widget_modal::{modal_view, ModalButton, ModalPalette, ModalSpec};
use llimphi_widget_menubar::{
    menubar_overlay_animated, menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H,
};
use llimphi_widget_nodegraph::{
    nodegraph_view_ex, NodeSpec, NodegraphMetrics, NodegraphPalette, Wire,
};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use llimphi_widget_text_editor::{EditorPalette as TEPalette, Language};
use llimphi_widget_text_input::{text_input_view, TextInputPalette};
use pluma_align::CartaHebras;
use pluma_cuerpo::Cuerpo;
use pluma_editor_llimphi::cuerpo_ide::CuerpoIde;
use pluma_editor_llimphi::multilienzo::PaletaHebras;
use pluma_editor_llimphi::lienzos::{
    lienzos_multi_view, ConfigLienzos, EdicionLienzo, EjecucionLienzo,
};
use pluma_editor_llimphi::multilienzo_editor::{
    multilienzo_editor_view_estilado, ConfigMultilienzoEditor,
};
use pluma_editor_llimphi::Palette as MultPalette;
use pluma_deck_recorrido_llimphi::recorrido_view;
use pluma_deck_outline::recorrido_desde_cuerpo;
use pluma_core::NarrativeAtom;
use pluma_transform::Transformacion;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::model::{
    ancho_contenido, Filtro, Modo, Model, Msg, ObjetivoEstilo, WizardTipo, ANCHO_COL, BACKENDS,
    METRICS, RAIL_W, VISIBLE_LINES,
};
use pluma_estilo::EstiloTexto;
use crate::update::{contar_stale_del_activo, menu_principal};
use crate::util::{etiqueta_backend, etiqueta_intencion, etiqueta_tipo, recortar};

/// Tamaño de ventana del init — usado como viewport para clampear los
/// dropdowns del menú (la app no trackea el tamaño real hoy).
const VIEWPORT: (f32, f32) = (1600.0, 900.0);

/// Icono vectorial y nombre de los cuatro dientes del rail. El icono lo pinta
/// `llimphi-icons` (mismo set canónico que cosmos — sin tofu); el nombre
/// completo va en la cabecera del panel.
/// Dientes del rail izquierdo (herramientas). "Derivar" fue reemplazado por el
/// wizard modal del diente "+" del rail derecho.
const DIENTES: [(Icon, &str); 4] = [
    (Icon::File, "Archivo"),
    (Icon::Folder, "Lienzos"),
    (Icon::Settings, "Modelo"),
    (Icon::Link, "Grafo"),
];

/// Arma el `MenuBarSpec` compartido entre `menubar_view` (barra) y
/// `menubar_overlay` (dropdown).
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

pub fn vista(model: &Model) -> View<Msg> {
    let theme = Theme::dark();
    let splitter_palette = SplitterPalette::from_theme(&theme);

    let menu = menu_principal(model);
    let menubar = menubar_view(&menubar_spec(&menu, model, &theme));
    let status = barra_status(model, &theme);

    // El panel del diente activo (izquierda, resizable) | el centro multilienzo.
    let panel = panel_diente(model, &theme);
    let centro = centro_multilienzo(model, &theme);

    // El rail flota como overlay pegado al borde INTERNO (el que da al centro),
    // dentro del área central — los dientes "sobresalen" del panel hacia el
    // centro, exactamente como cosmos (center_view + dock_rail_overlay). En
    // modo delegado lo dibuja pata, así que pluma no lo pinta.
    let mut centro_hijos: Vec<View<Msg>> = vec![centro];
    if !model.delegated {
        centro_hijos.push(rail_overlay(model, &theme));
        // Rail derecho: un diente por lienzo (estilo) + "+" (wizard).
        centro_hijos.push(rail_estilo_overlay(model, &theme));
    }
    let centro_con_rail = View::new(Style {
        position: Position::Relative,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(centro_hijos);

    let split = splitter_two(
        Direction::Row,
        panel,
        PaneSize::Fixed(model.panel_w),
        centro_con_rail,
        PaneSize::Flex,
        |phase, dx| match phase {
            DragPhase::Move => Some(Msg::ResizePanel(dx)),
            DragPhase::End => None,
        },
        &splitter_palette,
    );

    // Panel de estilo a la derecha como pane fijo, cuando hay un diente de
    // estilo activo (patrón cosmos: rail overlay + panel al costado).
    let core = match model.diente_estilo_activo {
        Some(id) => splitter_two(
            Direction::Row,
            split,
            PaneSize::Flex,
            panel_estilo(model, id, &theme),
            PaneSize::Fixed(model.panel_estilo_w),
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::ResizePanelEstilo(dx)),
                DragPhase::End => None,
            },
            &splitter_palette,
        ),
        None => split,
    };

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
    .children(vec![menubar, status, core])
}

/// El rail de dientes como overlay absoluto en el borde interno izquierdo.
fn rail_overlay(model: &Model, theme: &Theme) -> View<Msg> {
    let items: Vec<DockRailItem> = DIENTES
        .iter()
        .enumerate()
        .map(|(i, _)| DockRailItem {
            id: i as u64,
            active: i == model.diente_activo,
        })
        .collect();
    let rail = dock_rail_view::<Msg, _, _, _>(
        &items,
        RAIL_W,
        &DockRailPalette::from_theme(theme),
        |id, size, color| {
            let icono = DIENTES.get(id as usize).map(|d| d.0).unwrap_or(Icon::File);
            // Caja cuadrada centrada que contiene el icono vectorial (Absolute,
            // llena su contenedor Relative) — proporción uniforme sin importar
            // la forma del diente.
            View::<Msg>::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: percent(1.0_f32),
                },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .children(vec![View::<Msg>::new(Style {
                position: Position::Relative,
                size: Size {
                    width: length(size),
                    height: length(size),
                },
                ..Default::default()
            })
            .children(vec![icon_view::<Msg>(icono, color, 1.8)])])
        },
        |id| Msg::SelectDiente(id as usize),
        |_| None,
    );
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(6.0_f32),
            left: length(0.0_f32),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(RAIL_W),
            height: auto(),
        },
        ..Default::default()
    })
    .children(vec![rail])
}

/// Uuids de los lienzos visibles en el orden del tree — un diente por cada uno
/// en el rail derecho de estilo.
fn lienzos_visibles(model: &Model) -> Vec<Uuid> {
    if model.solo_activo {
        model.activo.into_iter().collect()
    } else {
        model
            .orden_lienzos
            .iter()
            .copied()
            .filter(|id| model.seleccionados.contains(id))
            .collect()
    }
}

/// Id reservado del diente "+" (wizard) en el rail derecho.
const DIENTE_MAS: u64 = u64::MAX;

/// Rail derecho: un diente por lienzo visible (abre su panel de estilo) + un
/// diente "+" que abre el wizard de transformación. Overlay absoluto pegado al
/// borde interno derecho del centro (espejo de `rail_overlay`).
fn rail_estilo_overlay(model: &Model, theme: &Theme) -> View<Msg> {
    let uuids = lienzos_visibles(model);
    let mut items: Vec<DockRailItem> = uuids
        .iter()
        .enumerate()
        .map(|(i, id)| DockRailItem {
            id: i as u64,
            active: model.diente_estilo_activo == Some(*id),
        })
        .collect();
    items.push(DockRailItem {
        id: DIENTE_MAS,
        active: model.wizard.is_some(),
    });

    let uuids_act = uuids.clone();
    let rail = dock_rail_view_side::<Msg, _, _, _>(
        &items,
        RAIL_W,
        DockRailSide::InnerRight,
        &DockRailPalette::from_theme(theme),
        |id, size, color| {
            let icono = if id == DIENTE_MAS { Icon::Plus } else { Icon::Font };
            View::<Msg>::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: percent(1.0_f32),
                },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .children(vec![View::<Msg>::new(Style {
                position: Position::Relative,
                size: Size {
                    width: length(size),
                    height: length(size),
                },
                ..Default::default()
            })
            .children(vec![icon_view::<Msg>(icono, color, 1.8)])])
        },
        move |id| {
            if id == DIENTE_MAS {
                Msg::AbrirWizard
            } else {
                match uuids_act.get(id as usize).copied() {
                    Some(uuid) => Msg::SelectDienteEstilo(uuid),
                    None => Msg::AbrirWizard,
                }
            }
        },
        |_| None,
    );
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(6.0_f32),
            right: length(0.0_f32),
            left: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(RAIL_W),
            height: auto(),
        },
        ..Default::default()
    })
    .children(vec![rail])
}

/// Paleta de swatches para los selectores de color del panel de estilo.
const SWATCHES: [[u8; 4]; 8] = [
    [235, 235, 235, 255], // claro
    [25, 25, 28, 255],    // oscuro
    [225, 84, 75, 255],   // rojo
    [238, 178, 53, 255],  // ámbar
    [94, 184, 124, 255],  // verde
    [120, 150, 220, 255], // azul
    [170, 130, 220, 255], // violeta
    [80, 190, 200, 255],  // turquesa
];

/// El `CuerpoIde` del lienzo `id`: el activo (editable) o un read-only.
fn ide_de<'a>(model: &'a Model, id: Uuid) -> Option<&'a CuerpoIde> {
    if model.activo == Some(id) {
        Some(&model.ide)
    } else {
        model.ides_ro.get(&id)
    }
}

/// Panel de estilo del lienzo `id` (pane derecho). Selector de objetivo
/// (Lienzo / Zona / Selección) + controles de color, fuente, tamaño y formato.
/// Cada control emite un delta `EstiloTexto` (`Msg::AplicarEstilo`).
fn panel_estilo(model: &Model, id: Uuid, theme: &Theme) -> View<Msg> {
    let palette_btn = ButtonPalette::from_theme(theme);
    let nombre = model
        .cuerpos
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.metadatos.nombre_legible.clone())
        .unwrap_or_else(|| "(lienzo)".to_string());
    let objetivo = model.objetivo_estilo;

    // Selector de objetivo.
    let obj_btn = |label: &str, sel: bool, o: ObjetivoEstilo| {
        let pal = if sel {
            ButtonPalette::from_theme(theme)
        } else {
            ButtonPalette {
                bg: theme.bg_panel_alt,
                bg_hover: theme.bg_button_hover,
                fg: theme.fg_muted,
                radius: palette_btn.radius,
            }
        };
        button_view::<Msg>(label, &pal, Msg::SetObjetivoEstilo(o))
    };
    let fila_obj = fila_botones(vec![
        obj_btn("Lienzo", matches!(objetivo, ObjetivoEstilo::Lienzo), ObjetivoEstilo::Lienzo),
        obj_btn(
            "Zona",
            matches!(objetivo, ObjetivoEstilo::Zona(_)),
            ObjetivoEstilo::Zona(0),
        ),
        obj_btn(
            "Selección",
            matches!(objetivo, ObjetivoEstilo::Seleccion),
            ObjetivoEstilo::Seleccion,
        ),
    ]);

    let mut hijos: Vec<View<Msg>> = vec![
        encabezado(&format!("estilo · {}", recortar(&nombre, 22)), theme),
        fila_obj,
    ];

    // Sub-selector de zona cuando el objetivo es Zona.
    if let ObjetivoEstilo::Zona(z_sel) = objetivo {
        let n = ide_de(model, id).map(|i| i.n_zonas()).unwrap_or(0);
        if n == 0 {
            hijos.push(pista_texto("este lienzo no tiene zonas", theme));
        } else {
            let mut botones: Vec<View<Msg>> = Vec::new();
            for z in 0..n.min(8) {
                let sel = z == z_sel;
                let pal = if sel {
                    ButtonPalette::from_theme(theme)
                } else {
                    ButtonPalette {
                        bg: theme.bg_panel_alt,
                        bg_hover: theme.bg_button_hover,
                        fg: theme.fg_muted,
                        radius: palette_btn.radius,
                    }
                };
                botones.push(button_view::<Msg>(
                    &format!("{}", z + 1),
                    &pal,
                    Msg::SetObjetivoEstilo(ObjetivoEstilo::Zona(z)),
                ));
            }
            hijos.push(fila_botones(botones));
        }
    }

    hijos.push(divider(theme));

    // Color de texto.
    hijos.push(encabezado("color de texto", theme));
    hijos.push(fila_swatches(false));
    // Color de fondo (resaltado).
    hijos.push(encabezado("resaltado", theme));
    hijos.push(fila_swatches(true));
    hijos.push(divider(theme));

    // Tamaño.
    hijos.push(encabezado("tamaño", theme));
    hijos.push(fila_botones(
        [11.0_f32, 13.0, 16.0, 20.0, 28.0]
            .iter()
            .map(|&px| {
                button_view::<Msg>(
                    &format!("{}", px as i32),
                    &palette_btn,
                    Msg::AplicarEstilo(EstiloTexto {
                        size_px: Some(px),
                        ..Default::default()
                    }),
                )
            })
            .collect(),
    ));

    // Fuente.
    hijos.push(encabezado("fuente", theme));
    let mk_fuente = |label: &str, fam: Option<&str>| {
        button_view::<Msg>(
            label,
            &palette_btn,
            Msg::AplicarEstilo(EstiloTexto {
                font_family: fam.map(|s| s.to_string()),
                ..Default::default()
            }),
        )
    };
    hijos.push(fila_botones(vec![
        mk_fuente("Sans", Some("sans-serif")),
        mk_fuente("Serif", Some("serif")),
        mk_fuente("Mono", Some("monospace")),
    ]));

    // Peso + formato.
    hijos.push(encabezado("formato", theme));
    let peso = |label: &str, w: f32| {
        button_view::<Msg>(
            label,
            &palette_btn,
            Msg::AplicarEstilo(EstiloTexto {
                weight: Some(w),
                ..Default::default()
            }),
        )
    };
    hijos.push(fila_botones(vec![peso("Normal", 400.0), peso("Negrita", 700.0)]));
    let flag = |label: &str, e: EstiloTexto| button_view::<Msg>(label, &palette_btn, Msg::AplicarEstilo(e));
    hijos.push(fila_botones(vec![
        flag("Itálica", EstiloTexto { italic: Some(true), ..Default::default() }),
        flag("Subrayado", EstiloTexto { underline: Some(true), ..Default::default() }),
        flag("Tachado", EstiloTexto { strikethrough: Some(true), ..Default::default() }),
    ]));
    hijos.push(divider(theme));
    hijos.push(fila_botones(vec![
        button_view::<Msg>("quitar formato", &palette_btn, Msg::EstiloReset),
        button_view::<Msg>("cerrar", &palette_btn, Msg::CerrarPanelEstilo),
    ]));

    let header = encabezado(&format!("· {} ·", objetivo.etiqueta()), theme);
    let cuerpo = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .children(hijos);

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
            height: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .clip(true)
    .children(vec![header, cuerpo])
}

/// Una fila de swatches de color: aplica `color_fg` (o `color_bg` si `bg`).
fn fila_swatches(bg: bool) -> View<Msg> {
    let mut botones: Vec<View<Msg>> = Vec::new();
    for c in SWATCHES.iter() {
        let mut rgba = *c;
        if bg {
            rgba[3] = 90; // resaltado translúcido
        }
        let delta = if bg {
            EstiloTexto { color_bg: Some(rgba), ..Default::default() }
        } else {
            EstiloTexto { color_fg: Some(rgba), ..Default::default() }
        };
        botones.push(
            View::new(Style {
                size: Size {
                    width: length(26.0_f32),
                    height: length(22.0_f32),
                },
                ..Default::default()
            })
            .fill(Color::from_rgba8(c[0], c[1], c[2], 255))
            .radius(4.0)
            .on_click(Msg::AplicarEstilo(delta)),
        );
    }
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0_f32),
        },
        gap: Size {
            width: length(5.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(botones)
}

/// Texto de pista pequeño (gris).
fn pista_texto(texto: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(16.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(texto.to_string(), 10.0, theme.fg_muted, Alignment::Start)
}

fn barra_status(model: &Model, theme: &Theme) -> View<Msg> {
    let nombre = model
        .activo
        .and_then(|id| model.cuerpos.iter().find(|c| c.id == id))
        .map(|c| c.metadatos.nombre_legible.clone())
        .unwrap_or_else(|| "(sin doc)".to_string());
    let n_sel = model.seleccionados.len();
    let backend = etiqueta_backend(BACKENDS[model.backend_idx]);
    let estado = if model.en_curso {
        "»"
    } else if model.ultimo_error.is_some() {
        "!"
    } else {
        "·"
    };
    let texto = format!(
        "pluma · [{}] · {nombre} · {n_sel} lienzo(s) · backend {backend} · {estado} {}  (Ctrl+M cambia modo)",
        model.modo.etiqueta(),
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

// ---------------------------------------------------------------------------
// Panel del diente activo
// ---------------------------------------------------------------------------

/// Despacha al panel del diente seleccionado. Todos comparten un contenedor
/// con padding izquierdo = `RAIL_W` para no quedar tapados por el rail.
fn panel_diente(model: &Model, theme: &Theme) -> View<Msg> {
    let interior = match model.diente_activo {
        0 => panel_archivo(model, theme),
        1 => panel_lienzos(model, theme),
        2 => panel_modelo(model, theme),
        _ => panel_grafo(model, theme),
    };
    let nombre = DIENTES
        .get(model.diente_activo)
        .map(|d| d.1)
        .unwrap_or("");
    let header = encabezado(nombre, theme);

    // El rail ya no se monta sobre el panel (vive en el centro), así que el
    // panel usa padding normal.
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
            height: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .clip(true)
    .children(vec![header, interior])
}

/// Encabezado de sección — texto pequeño top-aligned, sin centrado vertical.
fn encabezado(texto: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(18.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        texto.to_uppercase(),
        10.0,
        theme.fg_muted,
        Alignment::Start,
    )
}

fn panel_archivo(model: &Model, theme: &Theme) -> View<Msg> {
    let palette_btn = ButtonPalette::from_theme(theme);
    let palette_input = TextInputPalette::from_theme(theme);

    let nuevo = button_view::<Msg>("+  nuevo doc  (Ctrl+N)", &palette_btn, Msg::NuevoDoc);
    let guardar = button_view::<Msg>("guardar  (Ctrl+S)", &palette_btn, Msg::Guardar);

    let input = text_input_view::<Msg>(
        &model.path_input,
        "ruta .md o .docx (Esc para salir)",
        model.path_focused,
        &palette_input,
        Msg::FocusPath,
    );
    let fila = View::new(Style {
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
        button_view::<Msg>("abrir", &palette_btn, Msg::AbrirArchivo),
        button_view::<Msg>("exportar", &palette_btn, Msg::ExportarMd),
    ]);

    columna(vec![nuevo, guardar, divider(theme), encabezado("archivo", theme), input, fila])
}

/// Tree de lienzos: originales y sus derivadas, con toggle de selección
/// múltiple. Filas top-aligned, sin centrado ni márgenes verticales, glifos
/// con cobertura de fuente.
fn panel_lienzos(model: &Model, theme: &Theme) -> View<Msg> {
    // El tree se pinta en el orden maestro `orden_lienzos` (reordenable por
    // drag). Cada fila lleva su índice para el payload del arrastre.
    let mut filas: Vec<View<Msg>> = Vec::new();
    for (idx, id) in model.orden_lienzos.iter().enumerate() {
        if let Some(c) = model.cuerpos.iter().find(|c| c.id == *id) {
            let derivada = c.metadatos.intencion.es_derivada();
            filas.push(fila_lienzo(model, c, derivada, idx, theme));
        }
    }

    let lista = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        flex_grow: 1.0,
        gap: Size {
            width: length(0.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .children(filas);

    let pista = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(16.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        "click abre · cuadrito suma · arrastrá el grip para reordenar".to_string(),
        9.5,
        theme.fg_muted,
        Alignment::Start,
    );

    columna(vec![lista, pista])
}

/// Una fila del tree: [grip ⠿] [checkbox] [nombre · intención]. El grip se
/// arrastra para reordenar; el checkbox suma/saca del multilienzo; el nombre
/// abre (activa) el lienzo. Texto a una sola línea con elipsis. `idx` es la
/// posición en `orden_lienzos` (payload del drag).
fn fila_lienzo(model: &Model, c: &Cuerpo, derivada: bool, idx: usize, theme: &Theme) -> View<Msg> {
    let en_sel = model.seleccionados.contains(&c.id);
    let es_activo = model.activo == Some(c.id);

    // Grip arrastrable (drag source, payload = idx): seis puntitos pintados.
    // `draggable` con handler que no produce Msg — sólo transporta el payload.
    let grip_color = theme.fg_muted;
    let grip = View::new(Style {
        size: Size {
            width: length(14.0_f32),
            height: length(20.0_f32),
        },
        ..Default::default()
    })
    .draggable(|_phase, _dx, _dy| None::<Msg>)
    .drag_payload(idx as u64)
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, Circle};
        use llimphi_ui::llimphi_raster::peniko::Fill;
        let cx0 = rect.x as f64 + 5.0;
        let cx1 = rect.x as f64 + 9.0;
        let cy0 = rect.y as f64 + rect.h as f64 / 2.0 - 4.0;
        for r in 0..3 {
            let cy = cy0 + r as f64 * 4.0;
            for cx in [cx0, cx1] {
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    grip_color,
                    None,
                    &Circle::new((cx, cy), 1.1),
                );
            }
        }
    });

    // Checkbox pintado con `paint_with` → toda la celda (20×20) es clickeable
    // (no un cuadrito chico imposible de acertar), y nunca tofu. Caja con
    // borde; rellena con el acento cuando el lienzo está en el multilienzo.
    let accent = theme.accent;
    let borde = theme.border;
    let vacio = theme.bg_panel_alt;
    let checkbox = View::new(Style {
        size: Size {
            width: length(20.0_f32),
            height: length(20.0_f32),
        },
        ..Default::default()
    })
    .on_click(Msg::ToggleSeleccion(c.id))
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, RoundedRect, Stroke};
        use llimphi_ui::llimphi_raster::peniko::Fill;
        let s = 13.0_f64;
        let x = rect.x as f64 + (rect.w as f64 - s) / 2.0;
        let y = rect.y as f64 + (rect.h as f64 - s) / 2.0;
        let caja = RoundedRect::new(x, y, x + s, y + s, 3.0);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            if en_sel { accent } else { vacio },
            None,
            &caja,
        );
        scene.stroke(&Stroke::new(1.3), Affine::IDENTITY, borde, None, &caja);
    });

    let fg = if es_activo || en_sel {
        theme.fg_text
    } else {
        theme.fg_muted
    };
    let etiqueta = format!(
        "{} · {}",
        c.metadatos.nombre_legible,
        etiqueta_intencion(&c.metadatos.intencion)
    );
    // Sangría de las derivadas vía padding (no con caracteres), una sola línea.
    let nombre = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        flex_grow: 1.0,
        flex_shrink: 1.0,
        min_size: Size {
            width: length(0.0_f32),
            height: auto(),
        },
        padding: Rect {
            left: length(if derivada { 14.0_f32 } else { 2.0_f32 }),
            right: length(2.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(etiqueta, 12.0, fg, Alignment::Start)
    .ellipsis(1)
    .on_click(Msg::AbrirDoc(c.id));

    // El activo se distingue por fondo + barra de acento a la izquierda (3px).
    let fondo = if es_activo {
        theme.bg_panel_alt
    } else {
        theme.bg_panel
    };
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0_f32),
        },
        align_items: Some(AlignItems::Center),
        min_size: Size {
            width: length(0.0_f32),
            height: auto(),
        },
        gap: Size {
            width: length(2.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(fondo)
    .radius(4.0)
    // Drop target: soltar otra fila acá la reordena a esta posición.
    .on_drop(move |payload| Some(Msg::ReordenarLienzo(payload as usize, idx)))
    .drop_hover_fill(theme.accent)
    .children(vec![grip, checkbox, nombre])
}

/// Cuerpo del wizard modal de nueva transformación (diente "+"): define la
/// semántica — sobre QUÉ lienzo (madre) y QUÉ transformación (tipo + parámetro)
/// se aplica. Reusa `preset_input` como campo de parámetro y los presets
/// guardados (útiles para Reescribir).
fn wizard_body(model: &Model, theme: &Theme) -> View<Msg> {
    let w = match &model.wizard {
        Some(w) => w,
        None => return View::new(Style::default()),
    };
    let palette_btn = ButtonPalette::from_theme(theme);
    let palette_input = TextInputPalette::from_theme(theme);

    // 1) Sobre qué lienzo (madre). Botones de todos los cuerpos; el elegido
    //    queda resaltado.
    let mut filas_madre: Vec<View<Msg>> = Vec::new();
    for id in &model.orden_lienzos {
        if let Some(c) = model.cuerpos.iter().find(|c| c.id == *id) {
            let elegido = w.madre == Some(c.id);
            let etiqueta = format!(
                "{}  {}  ·  {}",
                if elegido { "●" } else { "○" },
                recortar(&c.metadatos.nombre_legible, 22),
                etiqueta_intencion(&c.metadatos.intencion),
            );
            filas_madre.push(
                View::new(Style {
                    size: Size {
                        width: percent(1.0_f32),
                        height: length(22.0_f32),
                    },
                    ..Default::default()
                })
                .text_aligned(
                    etiqueta,
                    12.0,
                    if elegido { theme.fg_text } else { theme.fg_muted },
                    Alignment::Start,
                )
                .on_click(Msg::WizardMadre(c.id)),
            );
        }
    }
    let lista_madre = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .children(filas_madre);

    // 2) Qué transformación (tipo). Botones segmentados.
    let tipo_btn = |t: WizardTipo| {
        let activo = w.tipo == t;
        let pal = if activo {
            ButtonPalette::from_theme(theme)
        } else {
            ButtonPalette {
                bg: theme.bg_panel_alt,
                bg_hover: theme.bg_button_hover,
                fg: theme.fg_muted,
                radius: ButtonPalette::from_theme(theme).radius,
            }
        };
        button_view::<Msg>(t.etiqueta(), &pal, Msg::WizardTipoSel(t))
    };
    let fila_tipo = fila_botones(vec![
        tipo_btn(WizardTipo::Traducir),
        tipo_btn(WizardTipo::Tono),
        tipo_btn(WizardTipo::Resumir),
        tipo_btn(WizardTipo::Reescribir),
    ]);

    // 3) Parámetro (significado según el tipo) — reusa preset_input.
    let input = text_input_view::<Msg>(
        &model.preset_input,
        w.tipo.placeholder(),
        model.preset_focused,
        &palette_input,
        Msg::FocusPreset,
    );

    let mut hijos: Vec<View<Msg>> = vec![
        encabezado("sobre qué lienzo", theme),
        lista_madre,
        divider(theme),
        encabezado("qué transformación", theme),
        fila_tipo,
        input,
    ];

    // Presets reutilizables (sobre todo para Reescribir): guardar + usar.
    hijos.push(fila_botones(vec![button_view::<Msg>(
        "+ guardar prompt como preset",
        &palette_btn,
        Msg::GuardarPreset,
    )]));
    if !model.presets.is_empty() {
        hijos.push(encabezado("presets", theme));
        for (i, preset) in model.presets.iter().enumerate() {
            hijos.push(fila_preset(i, preset, theme));
        }
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(8.0_f32),
        },
        ..Default::default()
    })
    .children(hijos)
}

/// Fila horizontal de botones con gap uniforme — helper de los paneles/wizard.
fn fila_botones(hijos: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
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
    .children(hijos)
}

/// Una fila de preset: [usar ▸ prompt] [✗ borrar].
fn fila_preset(i: usize, preset: &str, theme: &Theme) -> View<Msg> {
    let usar = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(
        format!("»  {}", recortar(preset, 30)),
        11.5,
        theme.fg_text,
        Alignment::Start,
    )
    .on_click(Msg::UsarPreset(i));

    let borrar = View::new(Style {
        size: Size {
            width: length(20.0_f32),
            height: length(20.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned("x".to_string(), 11.0, theme.fg_muted, Alignment::Center)
    .on_click(Msg::BorrarPreset(i));

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![usar, borrar])
}

/// Diente Modelo: cycler de backend + transformaciones LLM + tocar/regenerar +
/// hijas del activo + historial.
fn panel_modelo(model: &Model, theme: &Theme) -> View<Msg> {
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

    let etiqueta_back = format!("backend: {}  »", etiqueta_backend(BACKENDS[model.backend_idx]));
    let cycler = button_view::<Msg>(&etiqueta_back, pal_backend, Msg::CicloBackend);

    let etiqueta_solo = if model.solo_activo {
        "ver: sólo activo  (Ctrl+D)"
    } else {
        "ver: todos  (Ctrl+D)"
    };
    let solo_btn = button_view::<Msg>(etiqueta_solo, pal_backend, Msg::DiffToggle);

    let mk = |label: &str, m: Msg| button_view::<Msg>(label, pal, m);
    let botones: Vec<View<Msg>> = vec![
        mk("traducir  »  qu", Msg::PedirTraducir("qu".into())),
        mk("traducir  »  en", Msg::PedirTraducir("en".into())),
        mk("tono formal", Msg::PedirTono("formal".into())),
        mk("resumir 30p", Msg::PedirResumir(Some(30))),
    ];

    let n_stale = contar_stale_del_activo(model);
    let label_regen = format!("regenerar stale ({n_stale})");
    let tocar_btn = button_view::<Msg>("tocar madre", pal, Msg::TocarMadre);
    let regen_btn = button_view::<Msg>(&label_regen, pal, Msg::RegenerarStale);

    let mut hijos: Vec<View<Msg>> = vec![cycler, solo_btn, divider(theme)];
    hijos.extend(botones);
    hijos.push(tocar_btn);
    hijos.push(regen_btn);
    hijos.push(divider(theme));
    hijos.push(seccion_hijas(model, theme));

    columna(hijos)
}

fn seccion_hijas(model: &Model, theme: &Theme) -> View<Msg> {
    let activo = model.activo;
    let hijas: Vec<&Cuerpo> = model
        .cuerpos
        .iter()
        .filter(|c| c.metadatos.intencion.es_derivada() && c.metadatos.derivado_de == activo)
        .collect();

    let mut filas: Vec<View<Msg>> = vec![encabezado(&format!("hijas ({})", hijas.len()), theme)];
    for h in &hijas {
        // El idx para el drag es la posición real en el orden maestro.
        let idx = model
            .orden_lienzos
            .iter()
            .position(|x| *x == h.id)
            .unwrap_or(0);
        filas.push(fila_lienzo(model, h, true, idx, theme));
    }
    filas.push(divider(theme));
    filas.push(seccion_historial(model, theme));

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        flex_grow: 1.0,
        gap: Size {
            width: length(0.0_f32),
            height: length(3.0_f32),
        },
        ..Default::default()
    })
    .children(filas)
}

fn seccion_historial(model: &Model, theme: &Theme) -> View<Msg> {
    let cuerpo_de = |id: Uuid| model.cuerpos.iter().find(|c| c.id == id);
    let activo = model.activo;
    let mut filtradas: Vec<&Transformacion> = model
        .transformaciones
        .iter()
        .filter(|t| match activo {
            Some(id) => t.madre == id || t.hija == id,
            None => true,
        })
        .collect();
    filtradas.sort_by(|a, b| b.creada_en.cmp(&a.creada_en));

    let mut filas: Vec<View<Msg>> =
        vec![encabezado(&format!("historial ({})", filtradas.len()), theme)];
    for t in &filtradas {
        let madre = cuerpo_de(t.madre)
            .map(|c| c.metadatos.nombre_legible.as_str())
            .unwrap_or("?");
        let hija = cuerpo_de(t.hija)
            .map(|c| c.metadatos.nombre_legible.as_str())
            .unwrap_or("?");
        let etiqueta = format!(
            "{}  →  {}  ·  {}",
            recortar(madre, 16),
            recortar(hija, 16),
            etiqueta_tipo(&t.tipo),
        );
        filas.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(20.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(etiqueta, 11.0, theme.fg_muted, Alignment::Start)
            .on_click(Msg::AbrirDoc(t.hija)),
        );
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .children(filas)
}

// ---------------------------------------------------------------------------
// Diente Grafo: grafo semántico de filtros → línea de lienzo
// ---------------------------------------------------------------------------

/// Rótulo corto de un filtro, para el título del nodo y los botones.
pub(crate) fn etiqueta_filtro(f: &Filtro) -> String {
    match f {
        Filtro::Traducir(l) => format!("traducir → {l}"),
        Filtro::Tono(e) => format!("tono: {e}"),
        Filtro::Resumir(Some(n)) => format!("resumir ≈{n}p"),
        Filtro::Resumir(None) => "resumir".to_string(),
        Filtro::Concepto(t) if t.is_empty() => "concepto".to_string(),
        Filtro::Concepto(t) => format!("concepto: {t}"),
    }
}

/// Panel del diente Grafo: botonera para agregar filtros + input del término
/// Concepto + correr/limpiar, y debajo el grafo (nodegraph) del pipeline.
fn panel_grafo(model: &Model, theme: &Theme) -> View<Msg> {
    let palette_btn = ButtonPalette::from_theme(theme);
    let palette_input = TextInputPalette::from_theme(theme);

    let fila = |hijos: Vec<View<Msg>>| {
        View::new(Style {
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
        .children(hijos)
    };

    let fila_add = fila(vec![
        button_view::<Msg>("+ →qu", &palette_btn, Msg::GrafoAdd(Filtro::Traducir("qu".into()))),
        button_view::<Msg>("+ →en", &palette_btn, Msg::GrafoAdd(Filtro::Traducir("en".into()))),
        button_view::<Msg>("+ tono", &palette_btn, Msg::GrafoAdd(Filtro::Tono("formal".into()))),
        button_view::<Msg>("+ resumir", &palette_btn, Msg::GrafoAdd(Filtro::Resumir(Some(30)))),
    ]);

    // Filtro semántico Concepto: input del término + botón que lo agrega.
    let input = text_input_view::<Msg>(
        &model.grafo_input,
        "concepto: río, tensión… (filtra párrafos)",
        model.grafo_input_focused,
        &palette_input,
        Msg::FocusGrafo,
    );
    let input_wrap = View::new(Style {
        flex_grow: 1.0,
        flex_shrink: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        min_size: Size {
            width: length(0.0_f32),
            height: auto(),
        },
        ..Default::default()
    })
    .children(vec![input]);
    let termino = model.grafo_input.text().trim().to_string();
    let fila_concepto = fila(vec![
        input_wrap,
        button_view::<Msg>("+ concepto", &palette_btn, Msg::GrafoAdd(Filtro::Concepto(termino))),
    ]);

    let fila_run = fila(vec![
        button_view::<Msg>("generar línea  »", &palette_btn, Msg::GenerarLinea),
        button_view::<Msg>("limpiar", &palette_btn, Msg::GrafoLimpiar),
    ]);

    let pista = encabezado("grafo · arrastrá nodos · click derecho borra un filtro", theme);
    let canvas = grafo_canvas(model, theme);

    columna(vec![fila_add, fila_concepto, fila_run, divider(theme), pista, canvas])
}

/// El canvas del nodegraph: nodo fuente (lienzo activo) → un nodo por filtro
/// → nodo sumidero "nueva línea", unidos por cables. `NodeId`: 0 = fuente,
/// `i+1` = filtro `i`, `len+1` = sumidero.
fn grafo_canvas(model: &Model, theme: &Theme) -> View<Msg> {
    let palette = NodegraphPalette::from_theme(theme);
    let metrics = NodegraphMetrics::default();

    let nombre_activo = model
        .activo
        .and_then(|id| model.cuerpos.iter().find(|c| c.id == id))
        .map(|c| recortar(&c.metadatos.nombre_legible, 18))
        .unwrap_or_else(|| "(sin activo)".to_string());

    let n = model.grafo.len();
    let mut nodes: Vec<NodeSpec> = Vec::with_capacity(n + 2);
    let mut wires: Vec<Wire> = Vec::with_capacity(n + 1);

    nodes.push(NodeSpec {
        id: 0,
        label: format!("fuente: {nombre_activo}"),
        x: model.grafo_src.0,
        y: model.grafo_src.1,
        inputs: Vec::new(),
        outputs: vec!["línea".into()],
    });
    let mut prev: u32 = 0;
    for (i, nf) in model.grafo.iter().enumerate() {
        let id = (i + 1) as u32;
        nodes.push(NodeSpec {
            id,
            label: etiqueta_filtro(&nf.filtro),
            x: nf.x,
            y: nf.y,
            inputs: vec!["entra".into()],
            outputs: vec!["sale".into()],
        });
        wires.push(Wire {
            from_node: prev,
            from_output: 0,
            to_node: id,
            to_input: 0,
        });
        prev = id;
    }
    let sink = (n + 1) as u32;
    nodes.push(NodeSpec {
        id: sink,
        label: "→ nueva línea".into(),
        x: model.grafo_sink.0,
        y: model.grafo_sink.1,
        inputs: vec!["pipe".into()],
        outputs: Vec::new(),
    });
    wires.push(Wire {
        from_node: prev,
        from_output: 0,
        to_node: sink,
        to_input: 0,
    });

    let grafo = nodegraph_view_ex::<Msg, _, _, _>(
        &nodes,
        &wires,
        &palette,
        &metrics,
        |nid: u32, phase, dx, dy| Some(Msg::GrafoDrag(nid, phase, dx, dy)),
        |_a: u32, _ap: u16, _b: u32, _bp: u16| None,
        Some(move |nid: u32| {
            // Sólo los filtros (1..=n) se borran; fuente y sumidero no.
            if nid >= 1 && nid <= n as u32 {
                Some(Msg::GrafoDel(nid))
            } else {
                None
            }
        }),
    );

    View::new(Style {
        position: Position::Relative,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .clip(true)
    .children(vec![grafo])
}

// ---------------------------------------------------------------------------
// Centro: multilienzo de los lienzos seleccionados
// ---------------------------------------------------------------------------

/// Despacha el centro según el modo unificado: lienzos jerárquicos (editar
/// in-situ), presentar (deck) o el editor plano clásico.
fn centro_multilienzo(model: &Model, theme: &Theme) -> View<Msg> {
    match model.modo {
        Modo::Plano => centro_plano(model, theme),
        Modo::Lienzos => envolver_centro(model, centro_lienzos(model, theme)),
        Modo::Presentar => envolver_centro(model, centro_presentar(model, theme)),
    }
}

/// Envoltorio común para los modos nuevos: deja el hueco del rail a la izquierda
/// (los dientes sobresalen) y llena el alto. El modo Plano ya trae su propio
/// envoltorio con scroll/find.
fn envolver_centro(model: &Model, interior: View<Msg>) -> View<Msg> {
    let pad_rail = if model.delegated { 0.0 } else { RAIL_W };
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(pad_rail),
            right: length(pad_rail),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .clip(true)
    .children(vec![interior])
}

/// Modo Lienzos: el documento como cajas anidadas (títulos que contienen su
/// contenido), editable in-situ, multilienzo. Click en una caja abre su editor.
fn centro_lienzos(model: &Model, theme: &Theme) -> View<Msg> {
    let palette_lienzo = MultPalette::from_theme(theme);
    let editor_palette = TEPalette::default();
    let cfg = ConfigLienzos::default();

    let ids: Vec<Uuid> = if model.solo_activo {
        model.activo.into_iter().collect()
    } else {
        model
            .orden_lienzos
            .iter()
            .copied()
            .filter(|id| model.seleccionados.contains(id))
            .collect()
    };
    let cuerpos_sel: Vec<&Cuerpo> = ids
        .iter()
        .filter_map(|id| model.cuerpos.iter().find(|c| c.id == *id))
        .collect();
    if cuerpos_sel.is_empty() {
        return View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(palette_lienzo.bg_app);
    }
    let activo_idx = model
        .activo
        .and_then(|a| cuerpos_sel.iter().position(|c| c.id == a))
        .unwrap_or(0);

    let atoms: HashMap<Uuid, &NarrativeAtom> =
        model.atoms.iter().map(|(k, v)| (*k, v)).collect();

    let edicion = model.editando.as_ref().map(|(atom, state)| EdicionLienzo {
        atom: *atom,
        state,
        palette: &editor_palette,
        on_pointer: Arc::new(Msg::LienzoEditPointer)
            as Arc<dyn Fn(_) -> Msg + Send + Sync>,
    });

    let ejecucion = EjecucionLienzo {
        salidas: &model.salidas,
        on_run: Arc::new(Msg::EjecutarLienzo) as Arc<dyn Fn(_) -> Msg + Send + Sync>,
    };

    // Cartas entre columnas consecutivas → cintas Sankey (mismo criterio que el
    // modo Plano; sin carta se empareja por posición).
    let mut cartas_sel: Vec<Option<&CartaHebras>> = Vec::new();
    for w in cuerpos_sel.windows(2) {
        cartas_sel.push(carta_entre(model, w[0].id, w[1].id));
    }

    let multi = lienzos_multi_view::<Msg, _>(
        &cuerpos_sel,
        &atoms,
        &palette_lienzo,
        &cfg,
        activo_idx,
        None,
        edicion.as_ref(),
        Some(&ejecucion),
        &cartas_sel,
        Msg::LienzoSelect,
    );

    // Scroll vertical: contenedor relativo que recorta (vía envolver_centro) con
    // el contenido absoluto desplazado hacia arriba por `lienzos_scroll_y`. Los
    // átomos pintan en su posición desplazada, así que el registro de hebras y
    // las cintas siguen el scroll automáticamente.
    View::new(Style {
        position: Position::Relative,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(0.0_f32),
            top: length(-model.lienzos_scroll_y),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        ..Default::default()
    })
    .children(vec![multi])])
}

/// Modo Presentar: vuela por las secciones del documento con la cámara del deck
/// (tipo Prezi). Construye el recorrido desde el árbol del cuerpo activo.
fn centro_presentar(model: &Model, _theme: &Theme) -> View<Msg> {
    let activo = model
        .activo
        .and_then(|a| model.cuerpos.iter().find(|c| c.id == a));
    let Some(cuerpo) = activo else {
        return View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        });
    };
    let atoms = &model.atoms;
    let rec = recorrido_desde_cuerpo(cuerpo, |id| atoms.get(&id).map(|a| a.content.to_string()));
    let inner = recorrido_view::<Msg>(&rec, &model.recorrido_state);
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![inner])
}

fn centro_plano(model: &Model, theme: &Theme) -> View<Msg> {
    let editor_palette = TEPalette::default();
    let palette_lienzo = MultPalette::from_theme(theme);
    let paleta_hebras = PaletaHebras::default();

    // Lista de cuerpos a mostrar, EN EL ORDEN DEL TREE (`orden_lienzos`),
    // filtrada por los seleccionados. En modo "sólo activo" se recorta a una
    // columna. Así reordenar el tree reordena las columnas.
    let ids: Vec<Uuid> = if model.solo_activo {
        model.activo.into_iter().collect()
    } else {
        model
            .orden_lienzos
            .iter()
            .copied()
            .filter(|id| model.seleccionados.contains(id))
            .collect()
    };

    let mut cuerpos_sel: Vec<&Cuerpo> = Vec::new();
    let mut ides_sel: Vec<&CuerpoIde> = Vec::new();
    for id in &ids {
        let Some(cuerpo) = model.cuerpos.iter().find(|c| c.id == *id) else {
            continue;
        };
        let ide = if model.activo == Some(*id) {
            &model.ide
        } else if let Some(ro) = model.ides_ro.get(id) {
            ro
        } else {
            continue;
        };
        cuerpos_sel.push(cuerpo);
        ides_sel.push(ide);
    }

    if cuerpos_sel.is_empty() {
        return View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(editor_palette.bg);
    }

    let activo_idx = model
        .activo
        .and_then(|a| cuerpos_sel.iter().position(|c| c.id == a))
        .unwrap_or(0);

    // Cartas entre columnas consecutivas (bidireccional).
    let mut cartas_sel: Vec<Option<&CartaHebras>> = Vec::new();
    for w in cuerpos_sel.windows(2) {
        cartas_sel.push(carta_entre(model, w[0].id, w[1].id));
    }

    // ≥2 lienzos → columnas de ancho fijo (overflow → scroll horizontal). Un
    // solo lienzo → columna elástica que llena el centro.
    let n = cuerpos_sel.len();
    let fijo = n >= 2;
    let cfg = ConfigMultilienzoEditor {
        ancho_cuerpo: if fijo { Some(ANCHO_COL) } else { None },
        // Fluido vivo en los cauces: natas + glow corriendo con la fase que la
        // app avanza por tick (`Msg::FlujoTick`).
        mostrar_flujo: true,
        fase_flujo: model.fase_flujo,
        ..Default::default()
    };

    // El índice de columna que reporta el widget se traduce al Uuid del cuerpo
    // de esa columna — así el foco va al cuerpo correcto sin depender de que
    // el orden visible coincida con `seleccionados`.
    let ids_col: Vec<Uuid> = cuerpos_sel.iter().map(|c| c.id).collect();
    let estilos_sel: Vec<Option<&pluma_estilo::EstiloLienzo>> =
        ids_col.iter().map(|id| model.estilos.get(id)).collect();
    let ids_click = ids_col.clone();
    let ids_hover = ids_col;
    let hover_on = model.foco_por_hover;
    let mult = multilienzo_editor_view_estilado::<Msg, _, _>(
        &ides_sel,
        &cuerpos_sel,
        &cartas_sel,
        &estilos_sel,
        activo_idx,
        &editor_palette,
        &paleta_hebras,
        &palette_lienzo,
        &cfg,
        METRICS,
        VISIBLE_LINES,
        Language::Plain,
        move |i, ev| {
            let id = ids_click.get(i).copied().unwrap_or_default();
            Msg::MultiPointer(id, ev)
        },
        move |i| {
            if hover_on {
                ids_hover.get(i).copied().map(Msg::AbrirDoc)
            } else {
                None
            }
        },
    );

    let centro: View<Msg> = if fijo {
        // Envoltorio scrollable: contenedor relative que recorta; el interior
        // va absolute con left = -scroll_x (mismo patrón que el demo completo).
        let total_w = ancho_contenido(n);
        View::new(Style {
            position: Position::Relative,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(editor_palette.bg)
        .clip(true)
        .children(vec![View::new(Style {
            position: Position::Absolute,
            inset: Rect {
                left: length(-model.scroll_x),
                top: length(0.0_f32),
                right: auto(),
                bottom: auto(),
            },
            size: Size {
                width: length(total_w),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .children(vec![mult])])
    } else {
        mult
    };

    // El centro ocupa el alto disponible; la barra de scroll (si hay overflow)
    // va abajo, fija.
    let centro = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![centro]);

    // Find overlay (inline) arriba del centro cuando está visible.
    let mut hijos: Vec<View<Msg>> = Vec::new();
    if model.find_visible {
        hijos.push(barra_find(model));
    }
    hijos.push(centro);
    if fijo {
        if let Some(bar) = scrollbar_horizontal(model, theme, cuerpos_sel.len()) {
            hijos.push(bar);
        }
    }

    // Padding izquierdo = ancho del rail: los dientes sobresalen sobre el
    // borde del centro sin tapar la primera columna. Sin rail interno
    // (delegado) no hace falta.
    let pad_rail = if model.delegated { 0.0 } else { RAIL_W };
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(pad_rail),
            right: length(pad_rail),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(editor_palette.bg)
    .clip(true)
    .children(hijos)
}

/// Barra de scroll horizontal del multilienzo: track + thumb arrastrable.
/// `None` si el contenido cabe entero (sin overflow). El thumb refleja la
/// fracción visible y su posición; arrastrarlo desplaza el scroll.
fn scrollbar_horizontal(model: &Model, theme: &Theme, n_cols: usize) -> Option<View<Msg>> {
    let contenido = ancho_contenido(n_cols);
    let panel_estilo = if model.diente_estilo_activo.is_some() {
        model.panel_estilo_w
    } else {
        0.0
    };
    let centro = (model.viewport.0 - model.panel_w - RAIL_W * 2.0 - panel_estilo).max(1.0);
    if contenido <= centro + 1.0 {
        return None; // cabe entero, sin barra
    }
    let track_w = centro;
    let thumb_w = (centro / contenido * track_w).clamp(28.0, track_w);
    let max_scroll = (contenido - centro).max(1.0);
    let max_thumb = (track_w - thumb_w).max(1.0);
    let thumb_x = (model.scroll_x / max_scroll) * max_thumb;
    // Arrastre del thumb: dx de pantalla → dx de scroll (proporción inversa).
    let factor = max_scroll / max_thumb;

    let thumb = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(thumb_x),
            top: length(0.0_f32),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(thumb_w),
            height: length(7.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.accent)
    .radius(3.5)
    .draggable(move |phase, dx, _dy| match phase {
        DragPhase::Move => Some(Msg::ScrollHoriz(dx * factor)),
        DragPhase::End => None,
    });

    let track = View::new(Style {
        position: Position::Relative,
        size: Size {
            width: percent(1.0_f32),
            height: length(7.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.border)
    .radius(3.5)
    .children(vec![thumb]);

    Some(
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(13.0_f32),
            },
            padding: Rect {
                left: length(2.0_f32),
                right: length(8.0_f32),
                top: length(3.0_f32),
                bottom: length(3.0_f32),
            },
            ..Default::default()
        })
        .children(vec![track]),
    )
}

/// Busca la carta de hebras entre dos cuerpos, en cualquier orden.
fn carta_entre(model: &Model, a: Uuid, b: Uuid) -> Option<&CartaHebras> {
    model.cartas.iter().find(|c| {
        (c.cuerpo_a == Some(a) && c.cuerpo_b == Some(b))
            || (c.cuerpo_a == Some(b) && c.cuerpo_b == Some(a))
    })
}

fn barra_find(model: &Model) -> View<Msg> {
    let theme = Theme::dark();
    let palette_input = TextInputPalette::from_theme(&theme);
    let palette_btn = ButtonPalette::from_theme(&theme);

    let input = text_input_view::<Msg>(
        &model.find_input,
        "buscar (Enter siguiente · Shift+Enter previo · Esc cerrar)",
        true,
        &palette_input,
        Msg::FindToggle,
    );

    let total = model.find_matches.len();
    let pos = if total == 0 { 0 } else { model.find_idx + 1 };
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
    .text_aligned(format!("{pos}/{total}"), 12.0, theme.fg_muted, Alignment::Center);

    let prev = button_view::<Msg>("<", &palette_btn, Msg::FindAnterior);
    let next = button_view::<Msg>(">", &palette_btn, Msg::FindSiguiente);
    let cerrar = button_view::<Msg>("x", &palette_btn, Msg::FindClose);

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

// ---------------------------------------------------------------------------
// Helpers de layout
// ---------------------------------------------------------------------------

/// Columna vertical con gap estándar — el contenedor común de los paneles.
fn columna(hijos: Vec<View<Msg>>) -> View<Msg> {
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
    .children(hijos)
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

/// Overlay flotante: menú de edición contextual o dropdown del menú principal.
pub fn vista_overlay(model: &Model) -> Option<View<Msg>> {
    let theme = Theme::dark();
    // El wizard de transformación tiene prioridad: modal bloqueante.
    if model.wizard.is_some() {
        return Some(modal_view(ModalSpec {
            title: "Nueva transformación".to_string(),
            body: wizard_body(model, &theme),
            buttons: vec![
                ModalButton::cancel("Cancelar", Msg::CerrarWizard),
                ModalButton::primary("Crear", Msg::WizardConfirm),
            ],
            size: (520.0, 560.0),
            viewport: model.viewport,
            on_dismiss: Msg::CerrarWizard,
            palette: ModalPalette::from_theme(&theme),
        }));
    }
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
