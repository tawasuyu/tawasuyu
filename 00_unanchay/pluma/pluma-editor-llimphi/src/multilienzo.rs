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
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Cap, Join, Point, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Extend, Fill, Gradient};
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
    /// Tamaño de fuente del preview de párrafo dentro de cada bloque, en px.
    /// La cabecera de columna usa ~0.85 de este valor.
    pub font_size: f32,
    /// Si se pinta el **flujo** animado sobre las hebras frescas: pulsos
    /// brillantes que viajan de la columna madre a la hija, como corriente
    /// eléctrica o fluido recorriendo el haz. Opt-in: el default es `false`
    /// para que los renders estáticos queden idénticos.
    pub mostrar_flujo: bool,
    /// Fase del flujo en `[0, 1)`: dónde están los pulsos a lo largo de la
    /// hebra en este frame. La app la avanza ~`dt/periodo` por tick (vía
    /// `llimphi-motion`) y la envuelve con `rem_euclid(1.0)`. Sólo se usa
    /// cuando `mostrar_flujo` está activo.
    pub fase_flujo: f32,
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
            font_size: 13.0,
            mostrar_flujo: false,
            fase_flujo: 0.0,
        }
    }
}

impl MultilienzoConfig {
    /// Deriva una configuración escalada uniformemente del default. `escala`
    /// = 1.0 devuelve el default; > 1.0 agranda todo (bloques más altos,
    /// columnas más anchas, fuente mayor) de forma proporcional. Es el
    /// resorte del zoom: la app guarda un nivel de escala y reconstruye el
    /// `MultilienzoConfig` desde acá en cada frame.
    pub fn con_escala(escala: f32) -> Self {
        let e = escala.clamp(0.5, 3.0);
        let base = Self::default();
        Self {
            altura_atom: base.altura_atom * e,
            gap_atom: base.gap_atom * e,
            ancho_cuerpo: base.ancho_cuerpo * e,
            ancho_carril: base.ancho_carril * e,
            alto_header: base.alto_header * e,
            font_size: base.font_size * e,
            // Padding y grosor del trazo crecen más despacio que la caja:
            // un trazo proporcional al zoom se vería tosco al ampliar.
            padding_top: base.padding_top,
            grosor_hebra: base.grosor_hebra * (1.0 + (e - 1.0) * 0.5),
            mostrar_flujo: base.mostrar_flujo,
            fase_flujo: base.fase_flujo,
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
    /// Confianza del alineamiento en `[0, 1]`. Modula el grosor del trazo
    /// y el radio de los nodos en los extremos — una hebra fuerte se ve
    /// más sólida que una tentativa.
    fuerza: f32,
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
    multilienzo_view_resaltado::<Msg>(
        cuerpos, atoms, cartas, cfg, paleta_hebras, palette, "",
    )
}

/// Variante con resaltado de búsqueda transversal: cualquier átomo cuyo
/// `content` contenga `resaltar` (case-insensitive) se pinta con un
/// fondo distinto. Pasar `""` desactiva el resaltado (idéntico a
/// [`multilienzo_view`]).
pub fn multilienzo_view_resaltado<Msg: Clone + 'static>(
    cuerpos: &[&Cuerpo],
    atoms: &IndiceAtoms<'_>,
    cartas: &[Option<&CartaHebras>],
    cfg: &MultilienzoConfig,
    paleta_hebras: &PaletaHebras,
    palette: &Palette,
    resaltar: &str,
) -> View<Msg> {
    armar_multilienzo::<Msg>(
        cuerpos,
        atoms,
        cartas,
        cfg,
        paleta_hebras,
        palette,
        resaltar,
        &|_, _| None,
    )
}

/// Variante interactiva: además del resaltado, recibe un callback que
/// el runtime invoca al hacer click en cualquier bloque de átomo de
/// cualquier columna. El callback recibe `(i_cuerpo, atom_id)` — el
/// índice del cuerpo dentro del slice `cuerpos` (no su `branch_id`) y
/// el `Uuid` del átomo cliqueado — y produce el `Msg` que el caller
/// quiera disparar (típicamente: cambiar cuerpo activo + saltar caret
/// del IDE a ese átomo).
///
/// La cabecera de la columna (rótulo) **no** es clickeable; solo los
/// bloques de párrafo.
pub fn multilienzo_view_interactivo<Msg, F>(
    cuerpos: &[&Cuerpo],
    atoms: &IndiceAtoms<'_>,
    cartas: &[Option<&CartaHebras>],
    cfg: &MultilienzoConfig,
    paleta_hebras: &PaletaHebras,
    palette: &Palette,
    resaltar: &str,
    on_atom_click: F,
) -> View<Msg>
where
    Msg: Clone + 'static,
    F: Fn(usize, Uuid) -> Msg,
{
    armar_multilienzo::<Msg>(
        cuerpos,
        atoms,
        cartas,
        cfg,
        paleta_hebras,
        palette,
        resaltar,
        &|i, id| Some(on_atom_click(i, id)),
    )
}

/// Núcleo común: las variantes públicas se diferencian solo en si
/// pasan o no un handler de click por átomo. El handler se modela como
/// `&dyn Fn(usize, Uuid) -> Option<Msg>` — `None` significa "no
/// cablear `on_click` en ese bloque" (caso no interactivo).
fn armar_multilienzo<Msg: Clone + 'static>(
    cuerpos: &[&Cuerpo],
    atoms: &IndiceAtoms<'_>,
    cartas: &[Option<&CartaHebras>],
    cfg: &MultilienzoConfig,
    paleta_hebras: &PaletaHebras,
    palette: &Palette,
    resaltar: &str,
    on_atom_click: &dyn Fn(usize, Uuid) -> Option<Msg>,
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
        hijos.push(columna_cuerpo::<Msg>(
            c,
            i,
            atoms,
            cfg,
            palette,
            alto_contenido,
            resaltar,
            on_atom_click,
        ));
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
///
/// `i_cuerpo` es el índice de esta columna dentro del slice del caller;
/// se lo pasamos a `on_atom_click` para que el caller sepa **qué**
/// cuerpo recibió el click sin tener que re-buscar por `branch_id`.
fn columna_cuerpo<Msg: Clone + 'static>(
    cuerpo: &Cuerpo,
    i_cuerpo: usize,
    atoms: &IndiceAtoms<'_>,
    cfg: &MultilienzoConfig,
    palette: &Palette,
    alto_total: f32,
    resaltar: &str,
    on_atom_click: &dyn Fn(usize, Uuid) -> Option<Msg>,
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
    .text_aligned(header_text, (cfg.font_size * 0.85).max(9.0), palette.fg_muted, Alignment::Start);

    let mut bloques: Vec<View<Msg>> = Vec::with_capacity(cuerpo.orden.len());
    let resaltar_lc = if resaltar.is_empty() {
        String::new()
    } else {
        resaltar.to_lowercase()
    };
    for (i, atom_id) in cuerpo.orden.iter().enumerate() {
        let (preview, hit) = atoms
            .get(atom_id)
            .map(|a| {
                let p = preview_text(a);
                let hit = !resaltar_lc.is_empty()
                    && a.content.to_lowercase().contains(&resaltar_lc);
                (p, hit)
            })
            .unwrap_or_else(|| ("(átomo ausente)".to_string(), false));
        let y = cfg.padding_top + cfg.alto_header + i as f32 * (cfg.altura_atom + cfg.gap_atom);
        let click_msg = on_atom_click(i_cuerpo, *atom_id);
        bloques.push(bloque_atom::<Msg>(&preview, y, cfg, palette, hit, click_msg));
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
    hit_busqueda: bool,
    click_msg: Option<Msg>,
) -> View<Msg> {
    // Fondo destacado cuando el átomo matchea la búsqueda transversal.
    // Mezcla 30% del color accent con el bg_panel base — visible sin
    // ser estridente.
    let fondo = if hit_busqueda {
        mezclar(palette.bg_panel, palette.border_strong, 0.35)
    } else {
        palette.bg_panel
    };
    let mut v = View::new(Style {
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
    .fill(fondo)
    .radius(4.0)
    .text_aligned(preview.to_string(), cfg.font_size, palette.fg_text, Alignment::Start);
    if let Some(msg) = click_msg {
        v = v.on_click(msg);
    }
    v
}

/// Interpolación lineal de dos colores por componente RGBA. `t = 0`
/// devuelve `a`, `t = 1` devuelve `b`, intermedio el blend.
fn mezclar(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let ca = a.components;
    let cb = b.components;
    Color::new([
        ca[0] + (cb[0] - ca[0]) * t,
        ca[1] + (cb[1] - ca[1]) * t,
        ca[2] + (cb[2] - ca[2]) * t,
        ca[3] + (cb[3] - ca[3]) * t,
    ])
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
    let mostrar_flujo = cfg.mostrar_flujo;
    // Fase normalizada a [0, 1): el fluido avanza al subir; la app la hace
    // girar con `rem_euclid` desde un acumulador de tiempo.
    let fase = cfg.fase_flujo.rem_euclid(1.0) as f64;
    // Alto del bloque de párrafo: determina cuánto engrosa la cinta para
    // ocupar el cauce (banda Sankey, no línea).
    let altura_atom = cfg.altura_atom;

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
        for h in &hebras {
            // --- Geometría de la cinta: una banda Sankey rellena que engrosa
            // para ocupar el cauce. El alto de la banda en cada extremo es
            // proporcional a la fuerza del alineamiento (más fuerte = caudal
            // más ancho), recortado para no rebasar el bloque. Bordes
            // superior e inferior en curva-S con tangentes horizontales. ---
            let x0 = rect.x as f64;
            let x1 = (rect.x + rect.w) as f64;
            let dx = (x1 - x0) * 0.5;
            let yc_izq = (rect.y + h.y_izq) as f64;
            let yc_der = (rect.y + h.y_der) as f64;
            let media = ((altura_atom * 0.5) * (0.5 + 0.42 * h.fuerza.clamp(0.0, 1.0)))
                .clamp(5.0, altura_atom * 0.46) as f64;
            let (it, ib) = (yc_izq - media, yc_izq + media);
            let (dt, db) = (yc_der - media, yc_der + media);

            // Cinta cerrada: borde superior (S) → lado derecho → borde
            // inferior (S de vuelta) → close → relleno.
            let mut cinta = BezPath::new();
            cinta.move_to((x0, it));
            cinta.curve_to((x0 + dx, it), (x1 - dx, dt), (x1, dt));
            cinta.line_to((x1, db));
            cinta.curve_to((x1 - dx, db), (x0 + dx, ib), (x0, ib));
            cinta.close_path();

            // --- Cuerpo del cauce: relleno translúcido del color de la
            // sección. Es lo que se ve en reposo (Sankey estático). ---
            scene.fill(Fill::NonZero, Affine::IDENTITY, atenuar_alpha(h.color, 0.6), None, &cinta);

            // --- Borde nítido que define la orilla. Punteado si stale. ---
            let orilla = if h.punteada {
                Stroke::new(1.3).with_caps(Cap::Round).with_dashes(0.0, [6.0, 4.0])
            } else {
                Stroke::new(1.3).with_caps(Cap::Round).with_join(Join::Round)
            };
            scene.stroke(&orilla, Affine::IDENTITY, atenuar_alpha(h.color, 0.85), None, &cinta);

            // Las hebras stale no transmiten: cauce seco, sin fluido.
            if !mostrar_flujo || h.punteada {
                continue;
            }

            // --- Fluido 2D · 1. Frente de onda: un gradiente lineal
            // repetido cuyas bandas brillantes barren el cauce de la madre a
            // la hija. Rellenando la PROPIA cinta con el gradiente, queda
            // recortado a la banda sin clip aparte — el brillo cubre todo el
            // ancho del cauce, no una línea. ---
            const PER: f64 = 58.0; // largo de un período de banda, en px
            let sx = x0 - fase * PER; // corre con la fase → bandas hacia la hija
            let y_ref = (it + ib) * 0.5;
            let claro = aclarar(h.color, 0.55);
            let hueco = atenuar_alpha(claro, 0.0); // mismo tinte, transparente
            let onda = Gradient::new_linear(Point::new(sx, y_ref), Point::new(sx + PER, y_ref))
                .with_extend(Extend::Repeat)
                .with_stops([
                    (0.0_f32, hueco),
                    (0.38_f32, hueco),
                    (0.5_f32, claro),
                    (0.62_f32, hueco),
                    (1.0_f32, hueco),
                ]);
            scene.fill(Fill::NonZero, Affine::IDENTITY, &onda, None, &cinta);

            // --- Fluido 2D · 2. Líneas de corriente: filamentos finos a
            // varias fracciones del alto de la banda, cada uno una curva-S
            // con dash-offset que marcha con la fase. Dan la lectura laminar
            // del fluido recorriendo el cauce (no sólo un destello). ---
            const PERIODO_DASH: f64 = 16.0;
            let off = -fase * PERIODO_DASH;
            for frac in [0.26_f64, 0.5, 0.74] {
                let yi = it + (ib - it) * frac;
                let yd = dt + (db - dt) * frac;
                let mut linea = BezPath::new();
                linea.move_to((x0, yi));
                linea.curve_to((x0 + dx, yi), (x1 - dx, yd), (x1, yd));
                let trazo = Stroke::new(1.4)
                    .with_caps(Cap::Round)
                    .with_dashes(off, [3.0, 9.0]);
                scene.stroke(
                    &trazo,
                    Affine::IDENTITY,
                    atenuar_alpha(claro, 0.7),
                    None,
                    &linea,
                );
            }
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
            fuerza: h.fuerza,
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

/// Mezcla el color hacia el blanco por un factor `[0, 1]` y le pone alpha
/// pleno. Útil para el núcleo brillante de un pulso de flujo: conserva el
/// tinte de la hebra pero "enciende" la carga que la recorre.
fn aclarar(c: Color, hacia_blanco: f32) -> Color {
    let t = hacia_blanco.clamp(0.0, 1.0);
    let [r, g, b, _] = c.components;
    Color::new([
        r + (1.0 - r) * t,
        g + (1.0 - g) * t,
        b + (1.0 - b) * t,
        1.0,
    ])
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
    fn variante_interactiva_invoca_callback_por_cada_atomo() {
        use std::cell::RefCell;
        let (a, _atoms_a) = cuerpo_con_atomos("es", Intencion::Original, &["uno", "dos", "tres"]);
        let (b, _atoms_b) = cuerpo_con_atomos("qu", Intencion::Traduccion, &["huk", "iskay"]);
        let idx: IndiceAtoms = IndiceAtoms::new();
        let cuerpos: Vec<&Cuerpo> = vec![&a, &b];
        let cartas: Vec<Option<&CartaHebras>> = vec![None];
        let cfg = MultilienzoConfig::default();
        let paleta = PaletaHebras::default();
        let palette = Palette::default();

        let visitas: RefCell<Vec<(usize, Uuid)>> = RefCell::new(Vec::new());
        let _v: View<()> = multilienzo_view_interactivo(
            &cuerpos,
            &idx,
            &cartas,
            &cfg,
            &paleta,
            &palette,
            "",
            |i, id| {
                visitas.borrow_mut().push((i, id));
            },
        );

        // Cada átomo de cada columna debe haber producido una visita —
        // así sabemos que el cableado de `on_click` está pasando por la
        // ruta del callback (3 átomos de `a` + 2 de `b` = 5).
        let v = visitas.borrow();
        assert_eq!(v.len(), 5);
        let cuerpo_ids: Vec<usize> = v.iter().map(|(i, _)| *i).collect();
        assert_eq!(cuerpo_ids, vec![0, 0, 0, 1, 1]);
        // Los Uuid emitidos deben coincidir con el orden de los cuerpos.
        assert_eq!(v[0].1, a.orden[0]);
        assert_eq!(v[2].1, a.orden[2]);
        assert_eq!(v[3].1, b.orden[0]);
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
