//! `wawa-explorer-llimphi` — visor Llimphi de imágenes Wawa.
//!
//! Uso:
//!   wawa-explorer-llimphi <ruta.img>
//!
//! Renderea el grafo direccionado por contenido del disco Wawa: tree a la
//! izquierda con expand/collapse y selección, panel de detalle a la derecha
//! con header (hash + tamaño + aridad), hex preview de los primeros bytes
//! del payload y listado de hijos.
//!
//! Sin AoE en esta iteración. El crate `wawa-explorer-aoe` está disponible
//! para wirearlo (botón "fetch from peers" cuando un hash referenciado no
//! exista en la imagen local) en una pasada posterior.

use std::collections::HashSet;
use std::env;
use std::path::PathBuf;

use format::Hash;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_tree::{tree_view, TreePalette, TreeRow, TreeSpec};
use wawa_explorer_core::{short_hex, Disco};

#[derive(Clone)]
enum Msg {
    Toggle(Hash),
    Select(Hash),
}

struct Model {
    disco: Option<Disco>,
    source: PathBuf,
    error: Option<String>,
    expanded: HashSet<Hash>,
    selected: Option<Hash>,
    raices: Vec<Hash>,
}

struct Explorer;

impl App for Explorer {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "wawa-explorer"
    }

    fn initial_size() -> (u32, u32) {
        (1100, 720)
    }

    fn init(_: &Handle<Msg>) -> Model {
        let source = env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(""));
        if source.as_os_str().is_empty() {
            return Model {
                disco: None,
                source,
                error: Some("uso: wawa-explorer-llimphi <ruta.img>".into()),
                expanded: HashSet::new(),
                selected: None,
                raices: Vec::new(),
            };
        }
        match Disco::abrir(&source) {
            Ok(d) => {
                let raices = raices_de(&d);
                let selected = raices.first().copied();
                Model { disco: Some(d), source, error: None, expanded: HashSet::new(), selected, raices }
            }
            Err(e) => Model {
                disco: None,
                source,
                error: Some(e.to_string()),
                expanded: HashSet::new(),
                selected: None,
                raices: Vec::new(),
            },
        }
    }

    fn update(mut model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        match msg {
            Msg::Toggle(h) => {
                if !model.expanded.remove(&h) {
                    model.expanded.insert(h);
                }
            }
            Msg::Select(h) => {
                model.selected = Some(h);
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let palette = Palette::from_theme(&theme);

        let header = header_view(model, &palette);
        let main = main_view(model, &theme, &palette);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(palette.bg)
        .children(vec![header, main])
    }
}

/// Determina las raíces a mostrar en el tree top-level. Prioridad:
/// manifest > raíz > orphans (objetos sin padre conocido). Si el disco
/// está vacío, lista vacía.
fn raices_de(d: &Disco) -> Vec<Hash> {
    let mut raices = Vec::new();
    if let Some(h) = d.superbloque().manifiesto {
        raices.push(h);
    }
    if let Some(h) = d.superbloque().raiz {
        if !raices.contains(&h) {
            raices.push(h);
        }
    }
    raices
}

/// Aplana el árbol a partir de las raíces, respetando el set de expandidos.
fn filas_visibles(model: &Model) -> Vec<TreeRow<Msg>> {
    let Some(disco) = &model.disco else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for &raiz in &model.raices {
        emitir_subtree(disco, &model.expanded, model.selected, raiz, 0, &mut rows);
    }
    rows
}

fn emitir_subtree(
    disco: &Disco,
    expanded: &HashSet<Hash>,
    selected: Option<Hash>,
    hash: Hash,
    depth: usize,
    rows: &mut Vec<TreeRow<Msg>>,
) {
    let objeto = disco.objeto(&hash);
    let hijos: &[Hash] = objeto.map(|o| o.hijos.as_slice()).unwrap_or(&[]);
    let has_children = !hijos.is_empty();
    let expanded_aqui = expanded.contains(&hash);

    let etiqueta = match objeto {
        Some(o) => format!(
            "{}  ·  {} bytes  ·  {} hijos",
            short_hex(&hash),
            o.datos.len(),
            o.hijos.len()
        ),
        None => format!("{}  ·  (no en imagen)", short_hex(&hash)),
    };

    rows.push(TreeRow {
        label: etiqueta,
        depth,
        has_children,
        expanded: expanded_aqui,
        selected: selected == Some(hash),
        on_toggle: Msg::Toggle(hash),
        on_select: Msg::Select(hash),
    });

    if expanded_aqui {
        for &h in hijos {
            emitir_subtree(disco, expanded, selected, h, depth + 1, rows);
        }
    }
}

/// Paleta del explorer — slots semánticos sobre el Theme.
struct Palette {
    bg: Color,
    bg_panel: Color,
    fg_text: Color,
    fg_muted: Color,
    fg_error: Color,
}

impl Palette {
    fn from_theme(t: &Theme) -> Self {
        Self {
            bg: t.bg_app,
            bg_panel: t.bg_panel,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_error: t.fg_destructive,
        }
    }
}

use llimphi_ui::llimphi_raster::peniko::Color;

fn header_view(model: &Model, palette: &Palette) -> View<Msg> {
    let texto = match (&model.disco, &model.error) {
        (_, Some(e)) => format!("wawa-explorer · error: {e}"),
        (Some(d), None) => {
            let sb = d.superbloque();
            format!(
                "wawa-explorer · {}  ·  {} bytes  ·  v{}  ·  cursor sector {}  ·  {} objetos",
                model.source.display(),
                d.bytes_imagen(),
                sb.version,
                sb.cursor,
                d.cantidad_objetos()
            )
        }
        (None, None) => "wawa-explorer".to_string(),
    };
    let color = if model.error.is_some() { palette.fg_error } else { palette.fg_muted };

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .text_aligned(texto, 11.0, color, Alignment::Start)
}

fn main_view(model: &Model, theme: &Theme, palette: &Palette) -> View<Msg> {
    let tree_palette = TreePalette::from_theme(theme);
    let rows = filas_visibles(model);
    let tree = tree_view(TreeSpec { rows, row_height: 22.0, indent_px: 16.0, palette: tree_palette });

    let tree_panel = View::new(Style {
        size: Size { width: length(420.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .clip(true)
    .children(vec![tree]);

    let detail = detail_view(model, palette);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![tree_panel, detail])
}

fn detail_view(model: &Model, palette: &Palette) -> View<Msg> {
    let (titulo, cuerpo) = match (&model.disco, model.selected) {
        (Some(disco), Some(hash)) => match disco.objeto(&hash) {
            Some(o) => {
                let titulo = format!(
                    "objeto {}  ·  {} bytes  ·  {} hijos",
                    hex_completo(&hash),
                    o.datos.len(),
                    o.hijos.len()
                );
                let mut cuerpo = String::new();
                cuerpo.push_str("payload (primeros 256 bytes):\n\n");
                cuerpo.push_str(&hex_dump(&o.datos, 256));
                if !o.hijos.is_empty() {
                    cuerpo.push_str("\nhijos:\n");
                    for (i, h) in o.hijos.iter().enumerate() {
                        cuerpo.push_str(&format!("  {i:3}.  {}\n", short_hex(h)));
                    }
                }
                (titulo, cuerpo)
            }
            None => (
                format!("objeto {}", hex_completo(&hash)),
                "no presente en la imagen local (referenciado por un padre).\n\nposible siguiente paso: pedirlo a peers Wawa por AoE (wawa-explorer-aoe).".to_string(),
            ),
        },
        _ => ("(seleccioná un objeto del tree)".into(), String::new()),
    };

    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(titulo, 11.0, palette.fg_text, Alignment::Start);

    let body = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(6.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(cuerpo, 11.0, palette.fg_muted, Alignment::Start);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(palette.bg)
    .clip(true)
    .children(vec![header, body])
}

fn hex_completo(h: &Hash) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

/// Hex dump tipo `xxd`: 16 bytes por línea, offset a la izquierda, hex en
/// el medio. Cap a `max_bytes` para mantener el render barato.
fn hex_dump(bytes: &[u8], max_bytes: usize) -> String {
    let n = bytes.len().min(max_bytes);
    let mut out = String::new();
    for chunk_idx in 0..n.div_ceil(16) {
        let start = chunk_idx * 16;
        let end = (start + 16).min(n);
        out.push_str(&format!("  {start:04x}  "));
        for b in &bytes[start..end] {
            out.push_str(&format!("{b:02x} "));
        }
        out.push('\n');
    }
    if bytes.len() > max_bytes {
        out.push_str(&format!("  … ({} bytes más)\n", bytes.len() - max_bytes));
    }
    out
}

fn main() {
    llimphi_ui::run::<Explorer>();
}
