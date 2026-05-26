//! `multilienzo` — vista de cuerpos paralelos del mismo documento.
//!
//! Pinta N columnas (cuerpos) intercaladas con N−1 *carriles* angostos donde
//! se trazan las *hebras*: diagonales que conectan párrafos correspondientes
//! entre cuerpos consecutivos. Color y trazo codifican origen y frescura.
//!
//! Contrato con el caller:
//!   - `cuerpos`: la lista en orden de presentación (de izquierda a derecha).
//!   - `atoms`: índice por `Uuid` con los `NarrativeAtom`s referenciados.
//!     El multilienzo no resuelve por su cuenta — lo recibe ya armado.
//!   - `cartas`: `cartas[i]` es la carta entre `cuerpos[i]` y `cuerpos[i+1]`.
//!     `None` significa "no hay carta calculada todavía para ese par":
//!     no se pintan hebras en ese carril.
//!
//! La vista no maneja scroll explícito: si el contenido excede el rect que
//! le asigna taffy, se recorta. La integración con scroll horizontal vendrá
//! cuando llimphi-ui exponga primitivas de scroll dedicadas — por ahora el
//! ancho total del HStack se calcula y se devuelve al caller, que puede
//! envolverlo en su propio contenedor con `clip(true)` y desplazarlo.

use std::collections::HashMap;

use llimphi_ui::llimphi_layout::taffy::prelude::{
    auto, length, percent, FlexDirection, Position, Rect, Size, Style,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Stroke};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use uuid::Uuid;

use pluma_align::{CartaHebras, OrigenAlineamiento};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::Cuerpo;

use crate::Palette;

/// Configuración geométrica del multilienzo.
#[derive(Debug, Clone, Copy)]
pub struct MultilienzoConfig {
    /// Altura uniforme de cada bloque de párrafo, en px.
    pub altura_atom: f32,
    /// Separación vertical entre bloques dentro de una columna.
    pub gap_atom: f32,
    /// Ancho de cada columna de cuerpo.
    pub ancho_cuerpo: f32,
    /// Ancho del carril intermedio donde se pintan las hebras.
    pub ancho_carril: f32,
    /// Padding interno superior — desde donde empiezan los primeros átomos.
    pub padding_top: f32,
    /// Altura de la cabecera de cada columna (rótulo del cuerpo).
    pub alto_header: f32,
    /// Grosor del trazo de las hebras, en px.
    pub grosor_hebra: f32,
}

impl Default for MultilienzoConfig {
    fn default() -> Self {
        Self {
            altura_atom: 64.0,
            gap_atom: 10.0,
            ancho_cuerpo: 280.0,
            ancho_carril: 72.0,
            padding_top: 12.0,
            alto_header: 28.0,
            grosor_hebra: 2.0,
        }
    }
}

/// Paleta semántica de las hebras. Distinta del [`Palette`] del editor
/// porque codifica una dimensión propia: el origen del alineamiento.
#[derive(Debug, Clone, Copy)]
pub struct PaletaHebras {
    /// Origen [`OrigenAlineamiento::Derivado`] — la hebra más confiable: la
    /// emitió una transformación.
    pub derivada: Color,
    /// Origen [`OrigenAlineamiento::Embeddings`] — confianza calculada por
    /// un modelo. Su saturación se modula por la `fuerza` del alineamiento.
    pub embeddings: Color,
    /// Origen [`OrigenAlineamiento::Manual`] — la trazó un humano.
    pub manual: Color,
    /// Hebra stale (la madre cambió tras la última regeneración).
    /// Desaturada, mate.
    pub stale: Color,
}

impl Default for PaletaHebras {
    fn default() -> Self {
        Self {
            // verde — consistente con `tone_color(Valid)`
            derivada: Color::from_rgba8(94, 184, 124, 230),
            // azul de embeddings
            embeddings: Color::from_rgba8(96, 150, 220, 230),
            // ámbar — consistente con `tone_color(Pending)` (autoría humana = atención)
            manual: Color::from_rgba8(238, 178, 53, 230),
            // gris frío semitransparente
            stale: Color::from_rgba8(150, 150, 150, 140),
        }
    }
}

/// Índice rápido para resolver `Uuid → &NarrativeAtom`. El editor lo
/// construye desde su `NarrativeGraph`; el multilienzo lo consume sin
/// asumir su origen.
pub type IndiceAtoms<'a> = HashMap<Uuid, &'a NarrativeAtom>;

/// Datos pre-calculados de una hebra, listos para que la closure de
/// `paint_with` solo dibuje. Se calcula en CPU una vez por frame.
#[derive(Debug, Clone, Copy)]
struct HebraPintada {
    /// Posición vertical del punto izquierdo dentro del carril, en px
    /// relativos al rect del carril.
    y_izq: f32,
    /// Posición vertical del punto derecho.
    y_der: f32,
    /// Color final con alpha modulado por fuerza/stale.
    color: Color,
    /// Si la hebra va punteada (stale o baja confianza). Una sola variable
    /// porque el patrón es uniforme: 6 px on, 4 px off.
    punteada: bool,
}

/// Construye la vista multilienzo completa. El nodo raíz es un HStack con
/// el ancho exacto del contenido — el caller lo envuelve si necesita clip
/// o scroll.
///
/// Si `cuerpos` está vacío, devuelve un nodo vacío. Si `cartas` tiene
/// menos de `cuerpos.len()-1` entradas, los carriles faltantes quedan sin
/// hebras (no es un error: el caller puede ir agregando cartas).
pub fn multilienzo_view<Msg: Clone + 'static>(
    cuerpos: &[&Cuerpo],
    atoms: &IndiceAtoms<'_>,
    cartas: &[Option<&CartaHebras>],
    cfg: &MultilienzoConfig,
    paleta_hebras: &PaletaHebras,
    palette: &Palette,
) -> View<Msg> {
    if cuerpos.is_empty() {
        return View::new(Style::default());
    }

    let alto_max = cuerpos
        .iter()
        .map(|c| c.orden.len())
        .max()
        .unwrap_or(0);
    let alto_contenido = cfg.padding_top
        + cfg.alto_header
        + alto_max as f32 * (cfg.altura_atom + cfg.gap_atom);

    let mut hijos: Vec<View<Msg>> = Vec::with_capacity(cuerpos.len() * 2 - 1);
    for (i, c) in cuerpos.iter().enumerate() {
        hijos.push(columna_cuerpo::<Msg>(c, atoms, cfg, palette, alto_contenido));
        if i + 1 < cuerpos.len() {
            let carta = cartas.get(i).copied().flatten();
            let derecha = cuerpos[i + 1];
            hijos.push(carril_hebras::<Msg>(
                c,
                derecha,
                carta,
                cfg,
                paleta_hebras,
                palette,
                alto_contenido,
            ));
        }
    }

    let ancho_total = cuerpos.len() as f32 * cfg.ancho_cuerpo
        + (cuerpos.len().saturating_sub(1)) as f32 * cfg.ancho_carril;

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: length(ancho_total),
            height: length(alto_contenido),
        },
        ..Default::default()
    })
    .fill(palette.bg_app)
    .children(hijos)
}

/// Columna de un cuerpo: cabecera + lista vertical de bloques de párrafo.
fn columna_cuerpo<Msg: Clone + 'static>(
    cuerpo: &Cuerpo,
    atoms: &IndiceAtoms<'_>,
    cfg: &MultilienzoConfig,
    palette: &Palette,
    alto_total: f32,
) -> View<Msg> {
    let header_text = format!(
        "{} · {}",
        cuerpo.metadatos.nombre_legible,
        intencion_label(&cuerpo.metadatos.intencion)
    );

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(cfg.alto_header),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .text_aligned(header_text, 11.0, palette.fg_muted, Alignment::Start);

    let mut bloques: Vec<View<Msg>> = Vec::with_capacity(cuerpo.orden.len());
    for (i, atom_id) in cuerpo.orden.iter().enumerate() {
        let preview = atoms
            .get(atom_id)
            .map(|a| preview_text(a))
            .unwrap_or_else(|| "(átomo ausente)".to_string());
        let y = cfg.padding_top + cfg.alto_header + i as f32 * (cfg.altura_atom + cfg.gap_atom);
        bloques.push(bloque_atom::<Msg>(&preview, y, cfg, palette));
    }

    View::new(Style {
        position: Position::Relative,
        size: Size {
            width: length(cfg.ancho_cuerpo),
            height: length(alto_total),
        },
        ..Default::default()
    })
    .children({
        let mut v = vec![header];
        v.extend(bloques);
        v
    })
}

/// Bloque de un párrafo dentro de una columna: caja con preview de texto,
/// absolutamente posicionada para que las posiciones Y coincidan con las
/// que el carril usa al pintar hebras.
fn bloque_atom<Msg: Clone + 'static>(
    preview: &str,
    y: f32,
    cfg: &MultilienzoConfig,
    palette: &Palette,
) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(8.0_f32),
            top: length(y),
            right: length(8.0_f32),
            bottom: auto(),
        },
        size: Size {
            width: auto(),
            height: length(cfg.altura_atom),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .radius(4.0)
    .text_aligned(preview.to_string(), 13.0, palette.fg_text, Alignment::Start)
}

/// Carril entre dos columnas: nodo que pinta diagonales (hebras) con
/// `paint_with`. Pre-calcula posiciones en CPU; la closure solo dibuja.
fn carril_hebras<Msg: Clone + 'static>(
    izq: &Cuerpo,
    der: &Cuerpo,
    carta: Option<&CartaHebras>,
    cfg: &MultilienzoConfig,
    paleta: &PaletaHebras,
    _palette: &Palette,
    alto_total: f32,
) -> View<Msg> {
    let hebras = match carta {
        Some(c) => precomputar_hebras(izq, der, c, cfg, paleta),
        None => Vec::new(),
    };
    let grosor = cfg.grosor_hebra;

    let nodo = View::new(Style {
        size: Size {
            width: length(cfg.ancho_carril),
            height: length(alto_total),
        },
        ..Default::default()
    });
    if hebras.is_empty() {
        return nodo;
    }
    nodo.paint_with(move |scene, _ts, rect| {
        let stroke_solido = Stroke::new(grosor as f64);
        let stroke_punteado = Stroke::new(grosor as f64).with_dashes(0.0, [6.0, 4.0]);
        for h in &hebras {
            let mut path = BezPath::new();
            path.move_to((rect.x as f64, (rect.y + h.y_izq) as f64));
            path.line_to(((rect.x + rect.w) as f64, (rect.y + h.y_der) as f64));
            let s = if h.punteada { &stroke_punteado } else { &stroke_solido };
            scene.stroke(s, Affine::IDENTITY, h.color, None, &path);
        }
    })
}

/// Pre-calcula `HebraPintada`s para un par de cuerpos. Resuelve la
/// ambigüedad de orden de `Alineamiento` (atom_a/atom_b vs izq/der)
/// consultando en qué cuerpo vive cada átomo.
fn precomputar_hebras(
    izq: &Cuerpo,
    der: &Cuerpo,
    carta: &CartaHebras,
    cfg: &MultilienzoConfig,
    paleta: &PaletaHebras,
) -> Vec<HebraPintada> {
    let pos_izq: HashMap<Uuid, usize> = izq
        .orden
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, i))
        .collect();
    let pos_der: HashMap<Uuid, usize> = der
        .orden
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, i))
        .collect();
    let centro = |i: usize| -> f32 {
        cfg.padding_top
            + cfg.alto_header
            + i as f32 * (cfg.altura_atom + cfg.gap_atom)
            + cfg.altura_atom * 0.5
    };

    let mut out = Vec::with_capacity(carta.hebras.len());
    for h in &carta.hebras {
        // Resolver cuál atom va a la izquierda y cuál a la derecha.
        let (i_izq, i_der) = if let (Some(&a), Some(&b)) =
            (pos_izq.get(&h.atom_a), pos_der.get(&h.atom_b))
        {
            (a, b)
        } else if let (Some(&a), Some(&b)) =
            (pos_izq.get(&h.atom_b), pos_der.get(&h.atom_a))
        {
            (a, b)
        } else {
            // La hebra apunta a átomos ajenos a este par — ignorar.
            continue;
        };

        let (color_base, atenuar_por_fuerza) = if !h.fresco {
            (paleta.stale, false)
        } else {
            match &h.origen {
                OrigenAlineamiento::Derivado { .. } => (paleta.derivada, false),
                OrigenAlineamiento::Manual { .. } => (paleta.manual, false),
                OrigenAlineamiento::Embeddings { .. } => (paleta.embeddings, true),
            }
        };
        let color = if atenuar_por_fuerza {
            atenuar_alpha(color_base, h.fuerza)
        } else {
            color_base
        };

        out.push(HebraPintada {
            y_izq: centro(i_izq),
            y_der: centro(i_der),
            color,
            punteada: !h.fresco,
        });
    }
    out
}

/// Reduce el alpha de un color por un factor `[0, 1]`. Conserva los
/// componentes de color tal cual; solo modula transparencia. Útil para
/// modular la saturación visual de hebras según su `fuerza`.
fn atenuar_alpha(c: Color, factor: f32) -> Color {
    let f = factor.clamp(0.0, 1.0);
    let [r, g, b, a] = c.components;
    Color::new([r, g, b, a * f])
}

/// Rótulo corto y legible para cada variante de `Intencion`. La UI lo
/// muestra junto al `nombre_legible` del cuerpo en la cabecera de columna.
fn intencion_label(intencion: &pluma_cuerpo::Intencion) -> String {
    use pluma_cuerpo::Intencion;
    match intencion {
        Intencion::Original => "original".to_string(),
        Intencion::Traduccion => "traducción".to_string(),
        Intencion::Tono { etiqueta } => format!("tono: {etiqueta}"),
        Intencion::Resumen { palabras_objetivo: Some(n) } => format!("resumen ≈{n}p"),
        Intencion::Resumen { palabras_objetivo: None } => "resumen".to_string(),
        Intencion::Reescritura { .. } => "reescritura".to_string(),
        Intencion::Anotacion => "anotación".to_string(),
        Intencion::Custom { kind } => kind.clone(),
    }
}

/// Recorta el `content` del átomo a un preview de UNA línea aproximado.
/// Sin parley aquí — solo trunca por bytes (cuidando frontera UTF-8) y
/// sustituye saltos de línea por espacios.
fn preview_text(atom: &NarrativeAtom) -> String {
    const LIMITE: usize = 140;
    let mut s = atom.content.replace('\n', " ");
    if s.len() > LIMITE {
        // Recortar respetando UTF-8.
        let mut corte = LIMITE;
        while !s.is_char_boundary(corte) && corte > 0 {
            corte -= 1;
        }
        s.truncate(corte);
        s.push('…');
    }
    s
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use pluma_align::{alinear_uno_a_uno, OrigenAlineamiento};
    use pluma_cuerpo::Intencion;

    /// Helper: cuerpo + atoms vivos (los retiene el caller).
    fn cuerpo_con_atomos(branch: &str, intencion: Intencion, textos: &[&str]) -> (Cuerpo, Vec<NarrativeAtom>) {
        let mut c = Cuerpo::nuevo(branch, branch, intencion, 100);
        let atoms: Vec<NarrativeAtom> = textos
            .iter()
            .map(|t| NarrativeAtom::new(*t, branch))
            .collect();
        for a in &atoms {
            c.agregar(a.id, 101);
        }
        (c, atoms)
    }

    #[test]
    fn vacio_devuelve_vista_sin_panico() {
        let cfg = MultilienzoConfig::default();
        let paleta = PaletaHebras::default();
        let palette = Palette::default();
        let _v: View<()> = multilienzo_view(&[], &IndiceAtoms::new(), &[], &cfg, &paleta, &palette);
    }

    #[test]
    fn precomputar_hebras_resuelve_orden_atom_a_atom_b() {
        let (a, atoms_a) = cuerpo_con_atomos("es", Intencion::Original, &["uno", "dos"]);
        let (b, atoms_b) = cuerpo_con_atomos("qu", Intencion::Traduccion, &["huk", "iskay"]);
        // Carta con atom_a=es_id, atom_b=qu_id (orden natural).
        let carta_natural = alinear_uno_a_uno(
            &a, &b,
            OrigenAlineamiento::Derivado { transformacion: Uuid::new_v4(), timestamp: 1 },
        );
        let cfg = MultilienzoConfig::default();
        let paleta = PaletaHebras::default();
        let hebras_n = precomputar_hebras(&a, &b, &carta_natural, &cfg, &paleta);
        assert_eq!(hebras_n.len(), 2);

        // Misma carta pero invertida (atom_a=qu, atom_b=es). Debe seguir resolviendo
        // las posiciones correctamente al cuerpo izq/der.
        let mut carta_invertida = CartaHebras::nueva().con_par(b.id, a.id);
        for h in &carta_natural.hebras {
            let invertida = pluma_align::Alineamiento {
                id: h.id,
                atom_a: h.atom_b,
                atom_b: h.atom_a,
                fuerza: h.fuerza,
                origen: h.origen.clone(),
                fresco: h.fresco,
            };
            carta_invertida.agregar(invertida);
        }
        let hebras_i = precomputar_hebras(&a, &b, &carta_invertida, &cfg, &paleta);
        // Las posiciones y_izq/y_der deben ser las mismas, sin importar el orden
        // declarado en la carta. (Es robusto a la convención del caller.)
        assert_eq!(hebras_n.len(), hebras_i.len());
        for (n, i) in hebras_n.iter().zip(hebras_i.iter()) {
            assert!((n.y_izq - i.y_izq).abs() < 1e-3);
            assert!((n.y_der - i.y_der).abs() < 1e-3);
        }

        let _ = (atoms_a, atoms_b);
    }

    #[test]
    fn stale_pinta_punteada_y_color_stale() {
        let (a, atoms_a) = cuerpo_con_atomos("es", Intencion::Original, &["x"]);
        let (b, atoms_b) = cuerpo_con_atomos("qu", Intencion::Traduccion, &["y"]);
        let mut carta = alinear_uno_a_uno(
            &a, &b,
            OrigenAlineamiento::Embeddings { modelo: "iniy-1".into(), timestamp: 100 },
        );
        carta.hebras[0].fresco = false;

        let paleta = PaletaHebras::default();
        let hebras = precomputar_hebras(&a, &b, &carta, &MultilienzoConfig::default(), &paleta);
        assert_eq!(hebras.len(), 1);
        assert!(hebras[0].punteada);
        // Color stale (alpha bajo).
        assert!(hebras[0].color.components[3] < 0.6);
        let _ = (atoms_a, atoms_b);
    }

    #[test]
    fn embeddings_modulan_alpha_por_fuerza() {
        let (a, _atoms_a) = cuerpo_con_atomos("es", Intencion::Original, &["x"]);
        let (b, _atoms_b) = cuerpo_con_atomos("qu", Intencion::Traduccion, &["y"]);
        let mut carta = alinear_uno_a_uno(
            &a, &b,
            OrigenAlineamiento::Embeddings { modelo: "iniy-1".into(), timestamp: 100 },
        );
        carta.hebras[0].fuerza = 0.4;

        let paleta = PaletaHebras::default();
        let hebras = precomputar_hebras(&a, &b, &carta, &MultilienzoConfig::default(), &paleta);
        // El alpha debe ser ~0.4 del alpha base de paleta.embeddings.
        let a_base = paleta.embeddings.components[3];
        assert!((hebras[0].color.components[3] - a_base * 0.4).abs() < 1e-3);
    }

    #[test]
    fn preview_text_trunca_respetando_utf8() {
        let txt = "á".repeat(200); // cada `á` ocupa 2 bytes
        let atom = NarrativeAtom::new(&txt, "main");
        let p = preview_text(&atom);
        // No debe panicar y debe terminar en `…`.
        assert!(p.ends_with('…'));
        assert!(p.len() <= 144);
    }
}
