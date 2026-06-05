//! Vistas: rail de dientes (Archivo · Lienzos · Derivar · Modelo) + panel del
//! diente activo + centro multilienzo (N editores lado a lado con hebras de
//! color y scroll horizontal) + barra de status + overlay de find.

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Position, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_theme::Theme;
use llimphi_ui::{DragPhase, View};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_context_menu::{context_menu_view_ex, ContextMenuExtras};
use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};
use llimphi_widget_edit_menu::{self as editmenu, EditFlags};
use llimphi_widget_menubar::{
    menubar_overlay_animated, menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H,
};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use llimphi_widget_text_editor::{EditorPalette as TEPalette, Language};
use llimphi_widget_text_input::{text_input_view, TextInputPalette};
use pluma_align::CartaHebras;
use pluma_cuerpo::Cuerpo;
use pluma_editor_llimphi::cuerpo_ide::CuerpoIde;
use pluma_editor_llimphi::multilienzo::PaletaHebras;
use pluma_editor_llimphi::multilienzo_editor::{
    multilienzo_editor_view, ConfigMultilienzoEditor,
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

/// Ancho del rail de dientes, en px.
const RAIL_W: f32 = 46.0;

/// Ancho fijo de cada columna del multilienzo cuando hay ≥2 lienzos (habilita
/// overflow → scroll horizontal). Con 1 lienzo la columna es elástica y llena.
const ANCHO_COL: f32 = 360.0;

/// Nombres y letras de los cuatro dientes del rail. La letra es el icono (sin
/// tofu garantizado); el nombre completo va en la cabecera del panel.
const DIENTES: [(&str, &str); 4] = [
    ("A", "Archivo"),
    ("L", "Lienzos"),
    ("D", "Derivar"),
    ("M", "Modelo"),
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

pub(crate) fn vista(model: &Model) -> View<Msg> {
    let theme = Theme::dark();
    let splitter_palette = SplitterPalette::from_theme(&theme);

    let menu = menu_principal(model);
    let menubar = menubar_view(&menubar_spec(&menu, model, &theme));
    let status = barra_status(model, &theme);

    // El panel del diente activo (izquierda, resizable) | el centro multilienzo.
    let panel = panel_diente(model, &theme);
    let centro = centro_multilienzo(model, &theme);

    let split = splitter_two(
        Direction::Row,
        panel,
        PaneSize::Fixed(model.panel_w),
        centro,
        PaneSize::Flex,
        |phase, dx| match phase {
            DragPhase::Move => Some(Msg::ResizePanel(dx)),
            DragPhase::End => None,
        },
        &splitter_palette,
    );

    // El rail flota como overlay pegado al borde izquierdo (los dientes
    // "sobresalen" sobre el panel, como en cosmos). En modo delegado lo dibuja
    // pata, así que pluma no lo pinta.
    let mut body_hijos: Vec<View<Msg>> = vec![split];
    if !model.delegated {
        body_hijos.push(rail_overlay(model, &theme));
    }
    let body = View::new(Style {
        position: Position::Relative,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(body_hijos);

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
            let letra = DIENTES.get(id as usize).map(|d| d.0).unwrap_or("·");
            View::<Msg>::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: percent(1.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(letra.to_string(), size, color, Alignment::Center)
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
        "pluma · {nombre} · {n_sel} lienzo(s) · backend {backend} · {estado} {}",
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
        2 => panel_derivar(model, theme),
        _ => panel_modelo(model, theme),
    };
    let nombre = DIENTES
        .get(model.diente_activo)
        .map(|d| d.1)
        .unwrap_or("");
    let header = encabezado(nombre, theme);

    let pad_izq = if model.delegated { 10.0 } else { RAIL_W + 6.0 };
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(pad_izq),
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
    let mut originales: Vec<&Cuerpo> = Vec::new();
    let mut derivadas: Vec<&Cuerpo> = Vec::new();
    for c in &model.cuerpos {
        if c.metadatos.intencion.es_derivada() {
            derivadas.push(c);
        } else {
            originales.push(c);
        }
    }

    let mut filas: Vec<View<Msg>> = Vec::new();
    for orig in &originales {
        filas.push(fila_lienzo(model, orig, false, theme));
        for d in &derivadas {
            if d.metadatos.derivado_de == Some(orig.id) {
                filas.push(fila_lienzo(model, d, true, theme));
            }
        }
    }
    // Derivadas huérfanas (madre fuera de lista) al final.
    for d in &derivadas {
        let madre_presente = originales
            .iter()
            .any(|o| Some(o.id) == d.metadatos.derivado_de);
        if !madre_presente {
            filas.push(fila_lienzo(model, d, true, theme));
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
        "click abre · cuadrito suma al multilienzo".to_string(),
        9.5,
        theme.fg_muted,
        Alignment::Start,
    );

    columna(vec![lista, pista])
}

/// Una fila del tree: [toggle ▣/▢] [nombre · intención]. El toggle suma/saca
/// del multilienzo; el nombre abre (activa) el lienzo.
fn fila_lienzo(model: &Model, c: &Cuerpo, derivada: bool, theme: &Theme) -> View<Msg> {
    let en_sel = model.seleccionados.contains(&c.id);
    let es_activo = model.activo == Some(c.id);

    // Cajita de selección PINTADA (no glifo → sin riesgo de tofu): cuadrito
    // relleno con el acento cuando está en el multilienzo, sólo borde si no.
    let cuadro = View::new(Style {
        size: Size {
            width: length(11.0_f32),
            height: length(11.0_f32),
        },
        ..Default::default()
    })
    .fill(if en_sel { theme.accent } else { theme.bg_panel_alt })
    .radius(2.0);
    let toggle = View::new(Style {
        size: Size {
            width: length(20.0_f32),
            height: length(20.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .children(vec![cuadro])
    .on_click(Msg::ToggleSeleccion(c.id));

    let sangria = if derivada { "  » " } else { "" };
    let fg = if es_activo { theme.fg_text } else if en_sel { theme.fg_text } else { theme.fg_muted };
    let etiqueta = format!(
        "{sangria}{} · {}",
        recortar(&c.metadatos.nombre_legible, 22),
        etiqueta_intencion(&c.metadatos.intencion)
    );
    let nombre = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(etiqueta, 12.0, fg, Alignment::Start)
    .on_click(Msg::AbrirDoc(c.id));

    let fondo = if es_activo {
        theme.bg_panel_alt
    } else {
        theme.bg_panel
    };
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(4.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(fondo)
    .radius(3.0)
    .children(vec![toggle, nombre])
}

/// Diente Derivar-IA: input de prompt + botones (derivar/guardar preset) +
/// lista de presets reutilizables.
fn panel_derivar(model: &Model, theme: &Theme) -> View<Msg> {
    let palette_btn = ButtonPalette::from_theme(theme);
    let palette_input = TextInputPalette::from_theme(theme);

    let input = text_input_view::<Msg>(
        &model.preset_input,
        "prompt: reescribí el lienzo activo… (Esc sale)",
        model.preset_focused,
        &palette_input,
        Msg::FocusPreset,
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
        button_view::<Msg>("derivar alterno", &palette_btn, Msg::CrearAlterno),
        button_view::<Msg>("+ preset", &palette_btn, Msg::GuardarPreset),
    ]);

    let mut hijos: Vec<View<Msg>> = vec![input, fila, divider(theme), encabezado("presets", theme)];

    if model.presets.is_empty() {
        hijos.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(18.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(
                "guardá un prompt para reusarlo".to_string(),
                10.0,
                theme.fg_muted,
                Alignment::Start,
            ),
        );
    }
    for (i, preset) in model.presets.iter().enumerate() {
        hijos.push(fila_preset(i, preset, theme));
    }

    columna(hijos)
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
        filas.push(fila_lienzo(model, h, true, theme));
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
// Centro: multilienzo de los lienzos seleccionados
// ---------------------------------------------------------------------------

fn centro_multilienzo(model: &Model, theme: &Theme) -> View<Msg> {
    let editor_palette = TEPalette::default();
    let palette_lienzo = MultPalette::from_theme(theme);
    let paleta_hebras = PaletaHebras::default();

    // Lista de cuerpos a mostrar, alineada con sus ides (activo = vivo,
    // resto = read-only). En modo "sólo activo" se recorta a una columna.
    let ids: Vec<Uuid> = if model.solo_activo {
        model.activo.into_iter().collect()
    } else {
        model.seleccionados.clone()
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
        ..Default::default()
    };

    let mult = multilienzo_editor_view::<Msg, _>(
        &ides_sel,
        &cuerpos_sel,
        &cartas_sel,
        activo_idx,
        &editor_palette,
        &paleta_hebras,
        &palette_lienzo,
        &cfg,
        METRICS,
        VISIBLE_LINES,
        Language::Plain,
        |i, ev| Msg::MultiPointer(i, ev),
    );

    let centro: View<Msg> = if fijo {
        // Envoltorio scrollable: contenedor relative que recorta; el interior
        // va absolute con left = -scroll_x (mismo patrón que el demo completo).
        let total_w = n as f32 * ANCHO_COL + (n as f32 - 1.0) * cfg.ancho_carril;
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

    // Find overlay (inline) arriba del centro cuando está visible.
    let mut hijos: Vec<View<Msg>> = Vec::new();
    if model.find_visible {
        hijos.push(barra_find(model));
    }
    hijos.push(centro);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(editor_palette.bg)
    .clip(true)
    .children(hijos)
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
