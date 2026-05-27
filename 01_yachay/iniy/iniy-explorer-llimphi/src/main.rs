//! `iniy-explorer-llimphi` — visualiza el corpus de iniy en Llimphi.
//!
//! Lee la DB SQLite de iniy y muestra:
//! - Header con conteos del corpus.
//! - Lista de fuentes con su reputación (score derivado del grafo NLI).
//! - Lista de aserciones, cada una coloreada por su opinión dominante
//!   (verde=creencia, rojo=descreencia, gris=incertidumbre) y atribuida
//!   a su fuente efectiva.
//!
//! MVP feo: lectura única al arrancar, sin polling. Re-lanzar el binario
//! para ver actualizaciones tras correr `iniy nli` o `iniy extract` de nuevo.
//!
//! Path de la DB: env `INIY_DB` o `./iniy.db`.

use std::path::PathBuf;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_app_header::{app_header, AppHeaderPalette};
use llimphi_widget_banner::{banner_view, BannerKind};
use llimphi_widget_card::{card_view, CardOptions, CardPalette};

use iniy_core::{Asercion, AsercionId, FuenteId, Implicacion, Opinion};
use iniy_graph::GrafoCreencias;
use iniy_store::{AsercionAtribuida, FuenteResumen, Store};

const MAX_ASERCIONES_VISIBLES: usize = 60;
const ACCENT_CREENCIA: Color = Color::from_rgba8(0xa3, 0xbe, 0x8c, 0xff);     // verde
const ACCENT_DESCREENCIA: Color = Color::from_rgba8(0xbf, 0x61, 0x6a, 0xff);  // rojo
const ACCENT_INCERTIDUMBRE: Color = Color::from_rgba8(0x88, 0x88, 0x99, 0xff); // gris
const ACCENT_CITADA: Color = Color::from_rgba8(0xeb, 0xcb, 0x8b, 0xff);       // ámbar

#[derive(Clone)]
enum Msg {}

struct Model {
    db_path: PathBuf,
    error: Option<String>,
    aserciones: Vec<AsercionAtribuida>,
    fuentes: Vec<FuenteResumen>,
    reputaciones: std::collections::HashMap<FuenteId, f32>,
    n_implicaciones: usize,
    theme: Theme,
}

struct Explorer;

impl App for Explorer {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "iniy explorer"
    }

    fn initial_size() -> (u32, u32) {
        (1000, 700)
    }

    fn init(_handle: &Handle<Msg>) -> Model {
        let db_path = std::env::var("INIY_DB")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("iniy.db"));

        let theme = Theme::dark();

        match cargar_modelo(&db_path) {
            Ok((aserciones, fuentes, reputaciones, n_implicaciones)) => Model {
                db_path,
                error: None,
                aserciones,
                fuentes,
                reputaciones,
                n_implicaciones,
                theme,
            },
            Err(e) => Model {
                db_path,
                error: Some(e.to_string()),
                aserciones: Vec::new(),
                fuentes: Vec::new(),
                reputaciones: std::collections::HashMap::new(),
                n_implicaciones: 0,
                theme,
            },
        }
    }

    fn update(model: Model, _: Msg, _: &Handle<Msg>) -> Model {
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = model.theme;
        let header_text = format!(
            "iniy · {}  ·  {} fuentes  ·  {} aserciones  ·  {} relaciones",
            model.db_path.display(),
            model.fuentes.len(),
            model.aserciones.len(),
            model.n_implicaciones,
        );
        let header =
            app_header::<Msg>(header_text, Vec::new(), &AppHeaderPalette::from_theme(&theme));

        let mut chrome: Vec<View<Msg>> = vec![header];

        if let Some(err) = &model.error {
            chrome.push(banner_view::<Msg>(BannerKind::Error, err.clone()));
            return rama_columna(theme, chrome);
        }
        if model.aserciones.is_empty() {
            chrome.push(banner_view::<Msg>(
                BannerKind::Info,
                "corpus vacío — corre `iniy ingest <ruta>` y `iniy extract <doc-id>` antes",
            ));
            return rama_columna(theme, chrome);
        }

        let palette = CardPalette::from_theme(&theme);

        // Bloque "fuentes" — primera mitad horizontal del cuerpo.
        let fuentes_titulo = etiqueta_seccion("fuentes", theme.fg_muted);
        let mut fuentes_cards: Vec<View<Msg>> = vec![fuentes_titulo];
        for f in &model.fuentes {
            fuentes_cards.push(fuente_card(f, model.reputaciones.get(&f.fuente.id).copied(), &theme, &palette));
        }
        let panel_fuentes = panel_columna(theme, fuentes_cards);

        // Bloque "aserciones" — segunda mitad horizontal.
        let asercs_titulo = etiqueta_seccion("aserciones", theme.fg_muted);
        let mut aserc_cards: Vec<View<Msg>> = vec![asercs_titulo];
        for att in model.aserciones.iter().take(MAX_ASERCIONES_VISIBLES) {
            aserc_cards.push(asercion_card(att, &theme, &palette));
        }
        if model.aserciones.len() > MAX_ASERCIONES_VISIBLES {
            aserc_cards.push(
                texto_simple(
                    format!("… +{} más", model.aserciones.len() - MAX_ASERCIONES_VISIBLES),
                    11.0,
                    theme.fg_muted,
                ),
            );
        }
        let panel_asercs = panel_columna(theme, aserc_cards);

        let body = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            gap: Size { width: length(12.0_f32), height: length(0.0_f32) },
            padding: Rect {
                left: length(12.0_f32),
                right: length(12.0_f32),
                top: length(8.0_f32),
                bottom: length(8.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![panel_fuentes, panel_asercs]);

        chrome.push(body);
        rama_columna(theme, chrome)
    }
}

fn rama_columna(theme: Theme, children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(children)
}

fn panel_columna(theme: Theme, children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(0.5_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .clip(true)
    .children(children)
}

fn etiqueta_seccion(s: impl Into<String>, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(s, 11.0, color, Alignment::Start)
}

fn texto_simple(s: impl Into<String>, size: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(size + 6.0) },
        padding: Rect {
            left: length(2.0_f32),
            right: length(2.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(s, size, color, Alignment::Start)
}

fn accent_por_opinion(op: &Opinion) -> Color {
    if op.creencia >= op.descreencia && op.creencia >= op.incertidumbre {
        ACCENT_CREENCIA
    } else if op.descreencia >= op.incertidumbre {
        ACCENT_DESCREENCIA
    } else {
        ACCENT_INCERTIDUMBRE
    }
}

fn asercion_card(att: &AsercionAtribuida, theme: &Theme, palette: &CardPalette) -> View<Msg> {
    let op = &att.asercion.opinion_autoral;
    let accent = if att.citada { ACCENT_CITADA } else { accent_por_opinion(op) };

    let texto = texto_simple(
        truncar(&att.asercion.texto, 100),
        12.0,
        theme.fg_text,
    );

    let fuente_str = match &att.fuente {
        Some(f) => {
            let kind = f.kind.as_deref().map(|k| format!(" [{k}]")).unwrap_or_default();
            let cita = if att.citada { " (citada)" } else { "" };
            format!("{}{}{}  ·  {}", f.nombre, kind, cita, att.doc_titulo)
        }
        None => format!("(sin fuente)  ·  {}", att.doc_titulo),
    };
    let fuente_line = texto_simple(fuente_str, 10.0, theme.fg_muted);

    let op_line = texto_simple(
        format!("b={:.2}  d={:.2}  u={:.2}  ·  p̂={:.2}",
            op.creencia, op.descreencia, op.incertidumbre, op.probabilidad_esperada()),
        10.0,
        theme.fg_muted,
    );

    card_view::<Msg>(
        vec![texto, fuente_line, op_line],
        CardOptions { accent: Some(accent), ..Default::default() },
        palette,
    )
}

fn fuente_card(f: &FuenteResumen, reputacion: Option<f32>, theme: &Theme, palette: &CardPalette) -> View<Msg> {
    let kind = f.fuente.kind.as_deref().map(|k| format!(" [{k}]")).unwrap_or_default();
    let cabecera = texto_simple(
        format!("{}{}", f.fuente.nombre, kind),
        12.0,
        theme.fg_text,
    );
    let conteo = texto_simple(
        format!("{} docs  ·  {} aserciones", f.n_docs, f.n_aserciones),
        10.0,
        theme.fg_muted,
    );
    let mut hijos = vec![cabecera, conteo];
    let accent = if let Some(rep) = reputacion {
        hijos.push(texto_simple(
            format!("reputación: {:+.2}", rep),
            10.0,
            theme.fg_muted,
        ));
        if rep > 0.1 {
            ACCENT_CREENCIA
        } else if rep < -0.1 {
            ACCENT_DESCREENCIA
        } else {
            ACCENT_INCERTIDUMBRE
        }
    } else {
        ACCENT_INCERTIDUMBRE
    };
    card_view::<Msg>(
        hijos,
        CardOptions { accent: Some(accent), ..Default::default() },
        palette,
    )
}

fn truncar(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut o: String = s.chars().take(n).collect();
    o.push('…');
    o
}

fn cargar_modelo(
    db_path: &std::path::Path,
) -> anyhow::Result<(
    Vec<AsercionAtribuida>,
    Vec<FuenteResumen>,
    std::collections::HashMap<FuenteId, f32>,
    usize,
)> {
    let store = Store::abrir(db_path)?;
    let aserciones = store.cargar_aserciones_atribuidas_todas()?;
    let fuentes = store.listar_fuentes()?;
    let imps = store.cargar_implicaciones_todas()?;
    let reputaciones = calcular_reputaciones(&aserciones, &imps);
    let n = imps.len();
    Ok((aserciones, fuentes, reputaciones, n))
}

/// Cálculo de reputación duplicado del CLI (versión simplificada: solo el
/// score). Para que el explorer no dependa de iniy-cli.
fn calcular_reputaciones(
    todas: &[AsercionAtribuida],
    imps: &[Implicacion],
) -> std::collections::HashMap<FuenteId, f32> {
    use std::collections::HashMap;
    let asercion_a_fuente: HashMap<AsercionId, FuenteId> = todas.iter()
        .filter_map(|a| a.fuente.as_ref().map(|f| (a.asercion.id, f.id)))
        .collect();
    let mut apoyada: HashMap<FuenteId, u32> = HashMap::new();
    let mut contradicha: HashMap<FuenteId, u32> = HashMap::new();
    for imp in imps {
        let Some(&fa) = asercion_a_fuente.get(&imp.premisa) else { continue; };
        let Some(&fb) = asercion_a_fuente.get(&imp.hipotesis) else { continue; };
        if fa == fb { continue; }
        let rel = &imp.relacion;
        if rel.entailment > rel.contradiction && rel.entailment > 0.0 {
            *apoyada.entry(fb).or_default() += 1;
        } else if rel.contradiction > 0.0 {
            *contradicha.entry(fb).or_default() += 1;
        }
    }
    let mut out = HashMap::new();
    for fid in asercion_a_fuente.values().copied().collect::<std::collections::HashSet<_>>() {
        let a = *apoyada.get(&fid).unwrap_or(&0) as f32;
        let c = *contradicha.get(&fid).unwrap_or(&0) as f32;
        let total = a + c;
        let score = if total > 0.0 { (a - c) / total } else { 0.0 };
        out.insert(fid, score);
    }
    out
}

fn main() {
    llimphi_ui::run::<Explorer>();
}

// Silenciar warnings de imports no usados en este MVP.
#[allow(dead_code)]
fn _suppress_unused() {
    let _ = Asercion {
        id: AsercionId::nuevo(),
        doc_id: iniy_core::DocId::nuevo(),
        chunk_id: iniy_core::ChunkId::nuevo(),
        texto: String::new(),
        opinion_autoral: Opinion::vacua(0.5).unwrap(),
    };
    let _ = GrafoCreencias::nuevo();
}
