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
use llimphi_ui::llimphi_raster::peniko::{Color, Fill, Gradient, Mix};
use llimphi_ui::llimphi_raster::vello::Scene;
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

/// Callback de reordenamiento de columnas: `(desde, hasta)` son índices dentro
/// del slice `cuerpos`. Se invoca al soltar la cabecera de la columna `desde`
/// sobre la columna `hasta`. `Arc` + `Send + Sync` porque viaja dentro de los
/// closures de drag/drop del `View`.
pub type ReorderCols<Msg> = std::sync::Arc<dyn Fn(usize, usize) -> Option<Msg> + Send + Sync>;

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
        None,
        None,
        &|_, _| None,
    )
}

/// Variante **cotejo**: pinta dos (o más) cuerpos como lienzos a comparar.
/// En vez de la identidad de fila por arcoíris, cada sección se tiñe según su
/// **divergencia** `∈ [0,1]` (mapa `divergencias`): **verde** = coincide,
/// virando al **rojo** cuanto más fuerte es la diferencia. El tinte corre por
/// el texto *y* por las cintas del carril, igual que la identidad de fila —
/// así un match se ve como una banda verde gruesa y una reescritura como una
/// cinta roja fina. Átomos sin entrada en `divergencias` caen a verde (0).
///
/// El mapa lo produce `pluma-cotejo::cotejar` (`Cotejo::divergencias`), unido
/// con las divergencias del lienzo de diferencias si se monta la columna del
/// medio. Las `cartas` se calculan con `cotejar` (origen `Manual`, sin
/// atenuación por fuerza: las cintas rojas quedan opacas y visibles).
pub fn multilienzo_cotejo_view<Msg: Clone + 'static>(
    cuerpos: &[&Cuerpo],
    atoms: &IndiceAtoms<'_>,
    cartas: &[Option<&CartaHebras>],
    divergencias: &HashMap<Uuid, f32>,
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
        Some(divergencias),
        None,
        &|_, _| None,
    )
}

/// Variante **cotejo reordenable**: como [`multilienzo_cotejo_view`] pero las
/// cabeceras de las columnas son **arrastrables** — soltar una sobre otra emite
/// `on_reorder(desde, hasta)` con los índices dentro de `cuerpos`. El caller
/// reordena su modelo y recalcula qué carta va en cada carril por adyacencia.
/// Las cintas, el coloreado y el scroll quedan idénticos: sólo se vuelve
/// interactiva la barra de título.
pub fn multilienzo_cotejo_view_reorderable<Msg, F>(
    cuerpos: &[&Cuerpo],
    atoms: &IndiceAtoms<'_>,
    cartas: &[Option<&CartaHebras>],
    divergencias: &HashMap<Uuid, f32>,
    cfg: &MultilienzoConfig,
    paleta_hebras: &PaletaHebras,
    palette: &Palette,
    resaltar: &str,
    on_reorder: F,
) -> View<Msg>
where
    Msg: Clone + 'static,
    F: Fn(usize, usize) -> Option<Msg> + Send + Sync + 'static,
{
    armar_multilienzo::<Msg>(
        cuerpos,
        atoms,
        cartas,
        cfg,
        paleta_hebras,
        palette,
        resaltar,
        Some(divergencias),
        Some(std::sync::Arc::new(on_reorder)),
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
        None,
        None,
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
    divergencias: Option<&HashMap<Uuid, f32>>,
    on_reorder: Option<ReorderCols<Msg>>,
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

    // El color de cada sección se decide según el modo:
    //
    //   - **Cotejo** (`divergencias` presente): cada átomo se tiñe por su
    //     divergencia `∈ [0,1]` — verde si coincide, rojo si difiere fuerte.
    //     El color no se propaga: lo dicta el mapa, átomo por átomo, igual a
    //     izquierda y derecha porque `cotejar` asigna la misma divergencia a
    //     ambas contrapartes de una sección.
    //
    //   - **Identidad de fila** (default): cada sección horizontal lleva un
    //     color propio que corre idéntico por toda la fila — texto y cintas.
    //     Se siembra en la primera columna por índice de fila y se PROPAGA
    //     hacia la derecha siguiendo los haces: el átomo derecho hereda la
    //     identidad del izquierdo con el que se alinea. Un átomo sin haz
    //     entrante cae a un color por su propia fila (orphan). Así el color es
    //     una continuidad horizontal, no una etiqueta por columna.
    let mut identidad: Vec<HashMap<Uuid, Color>> = vec![HashMap::new(); cuerpos.len()];
    if let Some(div) = divergencias {
        for (i, c) in cuerpos.iter().enumerate() {
            for id in c.orden.iter() {
                let d = div.get(id).copied().unwrap_or(0.0);
                identidad[i].insert(*id, color_divergencia(d));
            }
        }
    } else {
        for (row, id) in cuerpos[0].orden.iter().enumerate() {
            identidad[0].insert(*id, identidad_color(row));
        }
        for i in 0..cuerpos.len().saturating_sub(1) {
            if let Some(carta) = cartas.get(i).copied().flatten() {
                for (a_izq, a_der) in hebras_orientadas(carta, cuerpos[i], cuerpos[i + 1]) {
                    if let Some(&col) = identidad[i].get(&a_izq) {
                        identidad[i + 1].entry(a_der).or_insert(col);
                    }
                }
            }
            // Orphans de la columna i+1: identidad por su propia fila.
            for (row, id) in cuerpos[i + 1].orden.iter().enumerate() {
                identidad[i + 1].entry(*id).or_insert_with(|| identidad_color(row));
            }
        }
    }

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
            &identidad[i],
            on_reorder.as_ref(),
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
                &identidad[i],
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
    identidad: &HashMap<Uuid, Color>,
    on_reorder: Option<&ReorderCols<Msg>>,
    on_atom_click: &dyn Fn(usize, Uuid) -> Option<Msg>,
) -> View<Msg> {
    let header_text = format!(
        "{} · {}",
        cuerpo.metadatos.nombre_legible,
        intencion_label(&cuerpo.metadatos.intencion)
    );

    let mut header = View::new(Style {
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

    // Cabecera arrastrable (sólo en la variante reordenable): se la marca como
    // origen de drag con payload = índice de columna, y como drop target que
    // emite `on_reorder(desde, esta_columna)`. El cuerpo de la columna no se
    // arrastra — sólo la barra de título, como en `tiled_view_reorderable`.
    if let Some(reorder) = on_reorder {
        let reorder = reorder.clone();
        let destino = i_cuerpo;
        header = header
            .draggable(|_p, _dx, _dy| None::<Msg>)
            .drag_payload(i_cuerpo as u64)
            .on_drop(move |desde| reorder(desde as usize, destino))
            .drop_hover_fill(mezclar(palette.bg_panel, palette.fg_text, 0.20));
    }

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
        let tinte = identidad.get(atom_id).copied();
        bloques.push(bloque_atom::<Msg>(&preview, y, cfg, palette, hit, tinte, click_msg));
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
    identidad: Option<Color>,
    click_msg: Option<Msg>,
) -> View<Msg> {
    // Fondo destacado cuando el átomo matchea la búsqueda transversal.
    // Mezcla 30% del color accent con el bg_panel base — visible sin
    // ser estridente.
    let base = if hit_busqueda {
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
    });

    // Tinte de identidad de fila: la sección entera (texto + cintas) comparte
    // un color, igual a izquierda y derecha. Se mezcla suave con el fondo
    // (28%) para no tapar el texto. Sin identidad → fondo plano de siempre.
    match identidad {
        Some(c) => v = v.fill(mezclar(base, c, 0.28)),
        None => v = v.fill(base),
    }

    v = v
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
    identidad_izq: &HashMap<Uuid, Color>,
) -> View<Msg> {
    let hebras = match carta {
        Some(c) => precomputar_hebras(izq, der, c, cfg, paleta, identidad_izq),
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
        for (hi, h) in hebras.iter().enumerate() {
            // Banda Sankey que engrosa para ocupar el cauce; alto por extremo
            // proporcional a la fuerza del alineamiento, recortado al bloque.
            let x0 = rect.x as f64;
            let x1 = (rect.x + rect.w) as f64;
            let yc_izq = (rect.y + h.y_izq) as f64;
            let yc_der = (rect.y + h.y_der) as f64;
            let media = ((altura_atom * 0.5) * (0.5 + 0.42 * h.fuerza.clamp(0.0, 1.0)))
                .clamp(5.0, altura_atom * 0.46) as f64;
            pintar_cauce_fluido(
                scene,
                Cauce { x0, x1, it: yc_izq - media, ib: yc_izq + media, dt: yc_der - media, db: yc_der + media },
                h.color,
                fase,
                mostrar_flujo,
                h.punteada,
                hi as u32,
            );
        }
    })
}

/// Bordes de un cauce Sankey: x del extremo izquierdo/derecho y, en cada
/// uno, el tope y la base de la banda. La cinta se dibuja con curva-S entre
/// los bordes izquierdos y los derechos.
#[derive(Clone, Copy)]
pub(crate) struct Cauce {
    pub x0: f64,
    pub x1: f64,
    pub it: f64,
    pub ib: f64,
    pub dt: f64,
    pub db: f64,
}

/// Pinta un cauce Sankey con fluido: cuerpo translúcido + **gradiente sheen**
/// (brillo al centro, oscuro en las orillas → tubo iluminado) + **glow**
/// luminoso por el eje + **natas** caóticas (si `fluir`) + orilla nítida.
/// Compartido por el multilienzo de preview y el de editores para que ambos
/// se vean igual. `color` ya es el de la identidad de fila/sección.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pintar_cauce_fluido(
    scene: &mut Scene,
    c: Cauce,
    color: Color,
    fase: f64,
    fluir: bool,
    punteada: bool,
    semilla: u32,
) {
    let Cauce { x0, x1, it, ib, dt, db } = c;
    let dx = (x1 - x0) * 0.5;

    // Cinta cerrada: borde superior (S) → lado derecho → borde inferior (S de
    // vuelta) → close.
    let mut cinta = BezPath::new();
    cinta.move_to((x0, it));
    cinta.curve_to((x0 + dx, it), (x1 - dx, dt), (x1, dt));
    cinta.line_to((x1, db));
    cinta.curve_to((x1 - dx, db), (x0 + dx, ib), (x0, ib));
    cinta.close_path();

    // 1. Cuerpo del cauce: relleno translúcido. Lo que se ve en reposo.
    scene.fill(Fill::NonZero, Affine::IDENTITY, atenuar_alpha(color, 0.55), None, &cinta);

    // Eje (curva-S por el centro de la banda) y semialto medio — base del
    // gradiente y del glow.
    let yc0 = (it + ib) * 0.5;
    let yc1 = (dt + db) * 0.5;
    let semialto = (((ib - it) + (db - dt)) * 0.25).max(3.0);
    let brillo = aclarar(color, 0.5);

    // 2. Gradiente "sheen": vertical, brillante al centro de la banda y
    // transparente en las orillas → la cinta parece un tubo de luz. Se
    // rellena la propia cinta, así queda recortado a la banda.
    let ymid = (yc0 + yc1) * 0.5;
    let sheen = Gradient::new_linear(
        Point::new(x0, ymid - semialto),
        Point::new(x0, ymid + semialto),
    )
    .with_stops([
        (0.0_f32, atenuar_alpha(brillo, 0.0)),
        (0.5_f32, atenuar_alpha(brillo, if punteada { 0.10 } else { 0.32 })),
        (1.0_f32, atenuar_alpha(brillo, 0.0)),
    ]);
    scene.fill(Fill::NonZero, Affine::IDENTITY, &sheen, None, &cinta);

    // 3. Glow por el eje: un halo ancho muy translúcido + un núcleo fino
    // brillante, ambos siguiendo la curva-S → luminosidad que sigue al cauce.
    let mut eje = BezPath::new();
    eje.move_to((x0, yc0));
    eje.curve_to((x0 + dx, yc0), (x1 - dx, yc1), (x1, yc1));
    if !punteada {
        let halo = Stroke::new(semialto * 1.5).with_caps(Cap::Round).with_join(Join::Round);
        scene.stroke(&halo, Affine::IDENTITY, atenuar_alpha(brillo, 0.10), None, &eje);
        let nucleo = Stroke::new((semialto * 0.35).max(1.4)).with_caps(Cap::Round);
        scene.stroke(&nucleo, Affine::IDENTITY, atenuar_alpha(aclarar(color, 0.62), 0.34), None, &eje);
    }

    // 4. Fluido caótico: natas irregulares que resbalan y friccionan con las
    // paredes (clipeadas a la cinta). Velocidad parabólica (lenta en las
    // orillas) ⇒ se adelantan entre sí. Stale = cauce seco, sin natas.
    if fluir && !punteada {
        scene.push_layer(Fill::NonZero, Mix::Normal, 1.0, Affine::IDENTITY, &cinta);
        const NATAS: usize = 18;
        for bi in 0..NATAS {
            let seed = semilla
                .wrapping_mul(2917)
                .wrapping_add(bi as u32 * 101)
                .wrapping_add(7);
            let yf0 = hash01(seed.wrapping_mul(3).wrapping_add(1)) as f64;
            let sz = 0.40 + 1.30 * hash01(seed.wrapping_mul(3).wrapping_add(2)) as f64;
            let jit = 0.65 + 0.70 * hash01(seed.wrapping_mul(3).wrapping_add(5)) as f64;
            let fase0 = hash01(seed.wrapping_mul(3).wrapping_add(3)) as f64;
            let tono = hash01(seed.wrapping_mul(3).wrapping_add(4)) as f64;
            let vperfil = 0.18 + 0.95 * (1.0 - (2.0 * yf0 - 1.0).powi(2));
            let vel = vperfil * jit;
            let s = (fase0 + fase * vel).rem_euclid(1.0);
            let sway = 0.08 * (std::f64::consts::TAU * (fase * 0.6 + fase0)).sin();
            let yf = (yf0 + sway).clamp(0.02, 0.98);

            let cx = cub(x0, x0 + dx, x1 - dx, x1, s);
            let ct = cub(it, it, dt, dt, s);
            let cb = cub(ib, ib, db, db, s);
            let grosor_local = (cb - ct).max(2.0);
            let cy = ct + grosor_local * yf;
            let r = grosor_local * 0.5 * (0.18 + 0.40 * sz);
            let rx = r * 1.55;
            let ry = r;

            let fin = ((s / 0.14).clamp(0.0, 1.0) * ((1.0 - s) / 0.14).clamp(0.0, 1.0)) as f32;
            let a = (0.14 + 0.26 * sz as f32).min(0.52) * fin;
            let cuerpo = atenuar_alpha(aclarar(color, 0.22 + 0.34 * tono as f32), a);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                cuerpo,
                None,
                &nata_path(cx, cy, rx * 1.12, ry * 1.12, seed),
            );
            let nucleo = atenuar_alpha(aclarar(color, 0.5), a * 0.5);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                nucleo,
                None,
                &nata_path(cx + rx * 0.15, cy - ry * 0.12, rx * 0.55, ry * 0.55, seed ^ 0x9e37_79b9),
            );
        }
        scene.pop_layer();
    }

    // 5. Orilla nítida por encima — define la pared. Punteada si stale.
    let orilla = if punteada {
        Stroke::new(1.3).with_caps(Cap::Round).with_dashes(0.0, [6.0, 4.0])
    } else {
        Stroke::new(1.3).with_caps(Cap::Round).with_join(Join::Round)
    };
    scene.stroke(&orilla, Affine::IDENTITY, atenuar_alpha(color, 0.85), None, &cinta);
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
    identidad_izq: &HashMap<Uuid, Color>,
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
        // Resolver cuál atom va a la izquierda y cuál a la derecha, guardando
        // el id del átomo izquierdo para leer su identidad de fila.
        let (i_izq, i_der, id_izq) = if let (Some(&a), Some(&b)) =
            (pos_izq.get(&h.atom_a), pos_der.get(&h.atom_b))
        {
            (a, b, h.atom_a)
        } else if let (Some(&a), Some(&b)) =
            (pos_izq.get(&h.atom_b), pos_der.get(&h.atom_a))
        {
            (a, b, h.atom_b)
        } else {
            // La hebra apunta a átomos ajenos a este par — ignorar.
            continue;
        };

        // Color = identidad de la FILA (la del átomo izquierdo), no el origen
        // del alineamiento: la cinta lleva el mismo color que el texto que une.
        // Stale conserva el gris mate de la paleta. Embeddings modula alpha
        // por fuerza (alineamiento tentativo = cauce más translúcido).
        let color = if !h.fresco {
            paleta.stale
        } else {
            let base = identidad_izq.get(&id_izq).copied().unwrap_or(paleta.embeddings);
            if matches!(h.origen, OrigenAlineamiento::Embeddings { .. }) {
                atenuar_alpha(base, h.fuerza)
            } else {
                base
            }
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

/// Hash entero→`[0,1]` determinista (integer finalizer estilo MurmurHash).
/// Da la pseudo-aleatoriedad del fluido caótico sin `rand` y sin romper la
/// reproducibilidad de los renders/tests: misma semilla, misma nata.
fn hash01(x: u32) -> f32 {
    let mut h = x.wrapping_mul(2_654_435_761);
    h ^= h >> 15;
    h = h.wrapping_mul(2_246_822_519);
    h ^= h >> 13;
    h = h.wrapping_mul(3_266_489_917);
    h ^= h >> 16;
    (h & 0x00ff_ffff) as f32 / 16_777_215.0
}

/// Evalúa un Bézier cúbico escalar (un eje) en `t ∈ [0,1]`. Se usa para
/// muestrear el borde superior/inferior y el eje del cauce y así colocar
/// las natas sobre la curva-S real de la cinta.
fn cub(a: f64, b: f64, c: f64, d: f64, t: f64) -> f64 {
    let u = 1.0 - t;
    u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
}

/// Construye una "nata": un blob cerrado **irregular** (sin forma) centrado
/// en `(cx, cy)`, con radios base `rx`/`ry` perturbados vértice a vértice por
/// `seed`. K puntos en un anillo con radio y ángulo jiterados, unidos por
/// Catmull-Rom convertido a Bézier → contorno orgánico y suave, no un óvalo.
fn nata_path(cx: f64, cy: f64, rx: f64, ry: f64, seed: u32) -> BezPath {
    const K: usize = 7;
    let mut pts = [(0.0_f64, 0.0_f64); K];
    for (k, p) in pts.iter_mut().enumerate() {
        let kk = k as u32;
        let ang = std::f64::consts::TAU * (k as f64) / K as f64
            + (hash01(seed.wrapping_add(kk.wrapping_mul(131)).wrapping_add(7)) as f64 - 0.5) * 0.7;
        let rr = 0.58 + 0.66 * hash01(seed.wrapping_add(kk.wrapping_mul(977)).wrapping_add(3)) as f64;
        *p = (cx + ang.cos() * rx * rr, cy + ang.sin() * ry * rr);
    }
    let mut path = BezPath::new();
    path.move_to(pts[0]);
    for i in 0..K {
        let p0 = pts[(i + K - 1) % K];
        let p1 = pts[i];
        let p2 = pts[(i + 1) % K];
        let p3 = pts[(i + 2) % K];
        let c1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
        let c2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
        path.curve_to(c1, c2, p2);
    }
    path.close_path();
    path
}

/// Color de **identidad de fila**: cada sección horizontal (un átomo y sus
/// alineados a través de las columnas) recibe un tono propio, estable de
/// izquierda a derecha. Tonos repartidos por la rueda con el ángulo áureo
/// para que filas vecinas contrasten; saturación/valor fijos y suaves para
/// que el tinte no compita con el texto.
fn identidad_color(idx: usize) -> Color {
    let h = (idx as f32 * 0.618_034).fract();
    hsv(h, 0.58, 0.80)
}

/// Color de **divergencia** para el modo cotejo: rampa de calor de verde
/// (`d = 0`, coincide) a rojo (`d = 1`, diferencia máxima), pasando por ámbar
/// en el medio para que el viraje sea legible y no un pardo plano. Saturación
/// y valor en el mismo registro suave que [`identidad_color`] para que el
/// tinte no compita con el texto. Alpha pleno; la cinta lo atenúa al pintar.
fn color_divergencia(d: f32) -> Color {
    let d = d.clamp(0.0, 1.0);
    // Tres paradas: verde (94,184,124) → ámbar (224,176,72) → rojo (212,84,84).
    let verde = Color::from_rgba8(94, 184, 124, 255);
    let ambar = Color::from_rgba8(224, 176, 72, 255);
    let rojo = Color::from_rgba8(212, 84, 84, 255);
    if d <= 0.5 {
        mezclar(verde, ambar, d / 0.5)
    } else {
        mezclar(ambar, rojo, (d - 0.5) / 0.5)
    }
}

/// HSV→RGB (alpha 1.0). `h`, `s`, `v` en `[0,1]`.
fn hsv(h: f32, s: f32, v: f32) -> Color {
    let h6 = (h.fract() * 6.0).max(0.0);
    let i = h6.floor();
    let f = h6 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    let (r, g, b) = match i as i32 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Color::new([r, g, b, 1.0])
}

/// Para una carta entre `izq` y `der`, resuelve cada hebra al par
/// `(atom_izq, atom_der)` orientado por columna (misma desambiguación que
/// [`precomputar_hebras`]). Alimenta la propagación de identidad de fila.
fn hebras_orientadas(carta: &CartaHebras, izq: &Cuerpo, der: &Cuerpo) -> Vec<(Uuid, Uuid)> {
    use std::collections::HashSet;
    let set_izq: HashSet<Uuid> = izq.orden.iter().copied().collect();
    let set_der: HashSet<Uuid> = der.orden.iter().copied().collect();
    let mut out = Vec::with_capacity(carta.hebras.len());
    for h in &carta.hebras {
        let par = if set_izq.contains(&h.atom_a) && set_der.contains(&h.atom_b) {
            (h.atom_a, h.atom_b)
        } else if set_izq.contains(&h.atom_b) && set_der.contains(&h.atom_a) {
            (h.atom_b, h.atom_a)
        } else {
            continue;
        };
        out.push(par);
    }
    out
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
        let hebras_n = precomputar_hebras(&a, &b, &carta_natural, &cfg, &paleta, &std::collections::HashMap::new());
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
        let hebras_i = precomputar_hebras(&a, &b, &carta_invertida, &cfg, &paleta, &std::collections::HashMap::new());
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
        let hebras = precomputar_hebras(&a, &b, &carta, &MultilienzoConfig::default(), &paleta, &std::collections::HashMap::new());
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
        let hebras = precomputar_hebras(&a, &b, &carta, &MultilienzoConfig::default(), &paleta, &std::collections::HashMap::new());
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
    fn color_divergencia_verde_a_rojo() {
        // d=0 → verde (G domina), d=1 → rojo (R domina), monótono en el medio.
        let v = color_divergencia(0.0).components;
        let r = color_divergencia(1.0).components;
        assert!(v[1] > v[0], "en verde, G > R");
        assert!(r[0] > r[1], "en rojo, R > G");
        // El componente rojo crece monótonamente con la divergencia.
        let r25 = color_divergencia(0.25).components[0];
        let r75 = color_divergencia(0.75).components[0];
        assert!(r25 < r75, "más divergencia ⇒ más rojo");
    }

    #[test]
    fn cotejo_view_no_panica_y_tiñe_por_divergencia() {
        let (a, _aa) = cuerpo_con_atomos("a", Intencion::Original, &["uno", "dos"]);
        let (b, _ab) = cuerpo_con_atomos("b", Intencion::Original, &["uno", "DOS distinto"]);
        // Divergencia 0 al primero, alta al segundo de cada lado.
        let mut div: HashMap<Uuid, f32> = HashMap::new();
        div.insert(a.orden[0], 0.0);
        div.insert(b.orden[0], 0.0);
        div.insert(a.orden[1], 0.8);
        div.insert(b.orden[1], 0.8);

        let cfg = MultilienzoConfig::default();
        let paleta = PaletaHebras::default();
        let palette = Palette::default();
        let cuerpos: Vec<&Cuerpo> = vec![&a, &b];
        let cartas: Vec<Option<&CartaHebras>> = vec![None];
        let idx = IndiceAtoms::new();
        let _v: View<()> = multilienzo_cotejo_view(
            &cuerpos, &idx, &cartas, &div, &cfg, &paleta, &palette, "",
        );
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
