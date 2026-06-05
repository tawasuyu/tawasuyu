//! `multilienzo_editor` — N editores reales de cuerpo lado-a-lado.
//!
//! Reemplazo del par "vista panorámica readonly arriba + IDE único
//! abajo" por **un solo plano**: cada cuerpo es un editor multi-párrafo
//! real en su propia columna, las hebras cruzan los carriles intermedios
//! entre columnas. Click en cualquier editor le da el foco (cambia el
//! cuerpo activo) y posiciona el caret en la línea cliqueada.
//!
//! Diseño:
//!
//!   ┌────────────────┬─────┬────────────────┬─────┬────────────────┐
//!   │ header cuerpo0 │     │ header cuerpo1 │     │ header cuerpo2 │
//!   ├────────────────┤  c  ├────────────────┤  c  ├────────────────┤
//!   │                │  a  │                │  a  │                │
//!   │  CuerpoIde 0   │  r  │  CuerpoIde 1   │  r  │  CuerpoIde 2   │
//!   │  (text-editor) │  r  │  (text-editor) │  r  │  (text-editor) │
//!   │                │  i  │                │  i  │                │
//!   │                │  l  │                │  l  │                │
//!   └────────────────┴─────┴────────────────┴─────┴────────────────┘
//!                       │                      │
//!                       │ hebras (paint_with)  │
//!
//! Las hebras se pintan en coordenadas vivas: la `y` de cada extremo
//! se calcula como `(line - scroll_offset) * line_height + line_height/2`
//! del editor correspondiente, así siguen al scroll real. Si un extremo
//! queda fuera del viewport vertical del carril, se clampea al borde
//! (efecto "asoma por arriba/abajo" hasta que el usuario scrollea ese
//! cuerpo a la vista). Cada cuerpo scrollea independientemente; sin
//! scroll sincronizado en este MVP — las hebras se desalinean cuando
//! los viewports divergen, que es exactamente el feedback visual que
//! le decimos al usuario.

use llimphi_ui::llimphi_layout::taffy::prelude::{
    auto, length, percent, FlexDirection, Rect, Size, Style,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Stroke};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_widget_text_editor::{
    EditorMetrics, EditorPalette, Language, PointerEvent,
};
use pluma_align::{CartaHebras, OrigenAlineamiento};
use pluma_cuerpo::Cuerpo;
use uuid::Uuid;

use crate::cuerpo_ide::{cuerpo_ide_view, CuerpoIde};
use crate::multilienzo::PaletaHebras;
use crate::Palette;

/// Configuración geométrica de la vista de editores lado-a-lado.
#[derive(Debug, Clone, Copy)]
pub struct ConfigMultilienzoEditor {
    /// Ancho del carril intermedio donde se pintan las hebras, en px.
    pub ancho_carril: f32,
    /// Altura del header (rótulo del cuerpo) sobre cada editor, en px.
    pub alto_header: f32,
    /// Grosor del trazo de las hebras, en px.
    pub grosor_hebra: f32,
    /// Padding (en px) que rodea cada editor cuando es el cuerpo activo
    /// — pintado con `palette.border_strong` para destacar el foco.
    pub grosor_foco: f32,
    /// Ancho de cada columna de editor. `None` = columnas elásticas
    /// (`flex_grow`) que se reparten el viewport — sin overflow horizontal.
    /// `Some(w)` = ancho fijo por columna: el HStack mide su ancho real
    /// (puede exceder el viewport) y el caller lo envuelve en un contenedor
    /// con `clip` + desplazamiento horizontal para scrollearlo.
    pub ancho_cuerpo: Option<f32>,
    /// Si `true`, cada sección (átomo) lleva una **banda de color de identidad**
    /// en su borde izquierdo, con el mismo color en todos los lienzos: la
    /// sección *i* es del color *i* en todas las columnas. Une las secciones
    /// por color entre lienzos sin depender de que exista una carta de hebras,
    /// y dentro de un mismo lienzo las divide (separadores) sin perder la
    /// unión (color). Las hebras, cuando hay carta, toman ese mismo color.
    pub colorear_secciones: bool,
}

impl Default for ConfigMultilienzoEditor {
    fn default() -> Self {
        Self {
            ancho_carril: 56.0,
            alto_header: 28.0,
            grosor_hebra: 2.0,
            grosor_foco: 2.0,
            ancho_cuerpo: None,
            colorear_secciones: true,
        }
    }
}

/// Color de identidad de la sección `i` — paleta fija de tonos bien
/// distinguibles que cicla. El mismo `i` da el mismo color en cualquier
/// columna, que es lo que "une por color" las secciones entre lienzos.
pub fn color_seccion(i: usize) -> Color {
    // 8 tonos saturados pero no estridentes (estilo "category10" recortado).
    const PALETA: [(u8, u8, u8); 8] = [
        (94, 184, 124),  // verde
        (96, 150, 220),  // azul
        (238, 178, 53),  // ámbar
        (208, 110, 196), // magenta
        (96, 200, 200),  // cian
        (230, 120, 100), // coral
        (170, 150, 235), // lavanda
        (190, 200, 90),  // lima
    ];
    let (r, g, b) = PALETA[i % PALETA.len()];
    Color::from_rgba8(r, g, b, 255)
}

/// Datos pre-calculados de una **cinta** (ribbon Sankey) entre dos editores
/// vivos: el rango vertical `[top, bot]` que ocupa la sección en cada lado
/// (ya considera `scroll_offset` y `alto_header`), y su color. La cinta se
/// rellena con borde superior e inferior en curva-S (tangentes horizontales),
/// igual que `pineal-flow::ribbon`.
#[derive(Debug, Clone, Copy)]
struct HebraEditor {
    izq_top: f32,
    izq_bot: f32,
    der_top: f32,
    der_bot: f32,
    color: Color,
}

/// Render principal: N editores en HStack con carriles de hebras entre
/// cada par consecutivo.
///
/// Contrato:
///   - `ides[i]` corresponde a `cuerpos[i]`. El caller mantiene la
///     correspondencia 1↔1.
///   - `cartas[i]` es la carta entre `cuerpos[i]` y `cuerpos[i+1]`. `None`
///     deja el carril vacío.
///   - `activo` es el índice del cuerpo con foco — recibe un borde accent
///     visible. Si está fuera de rango, ningún editor se destaca.
///   - `on_pointer(i, ev)` se invoca para clicks/drag dentro del editor
///     `i`. El caller convierte `(x, y)` a `(line, col)` con
///     `metrics.screen_to_pos(x, y, scroll_offset)` y aplica al ide
///     correspondiente.
///
/// El nodo raíz mide ancho fijo (suma de columnas + carriles) y `height
/// = percent(1.0)` — el caller lo envuelve si quiere darle un tamaño
/// concreto.
pub fn multilienzo_editor_view<Msg, FPtr>(
    ides: &[&CuerpoIde],
    cuerpos: &[&Cuerpo],
    cartas: &[Option<&CartaHebras>],
    activo: usize,
    palette_editor: &EditorPalette,
    paleta_hebras: &PaletaHebras,
    palette_lienzo: &Palette,
    cfg: &ConfigMultilienzoEditor,
    metrics: EditorMetrics,
    visible_lines: usize,
    language: Language,
    on_pointer: FPtr,
) -> View<Msg>
where
    Msg: Clone + 'static,
    FPtr: Fn(usize, PointerEvent) -> Msg + Send + Sync + Clone + 'static,
{
    assert_eq!(
        ides.len(),
        cuerpos.len(),
        "multilienzo_editor: ides y cuerpos deben tener el mismo largo"
    );
    if ides.is_empty() {
        return View::new(Style::default());
    }

    let mut hijos: Vec<View<Msg>> = Vec::with_capacity(ides.len() * 2 - 1);
    for i in 0..ides.len() {
        let on_pointer_i = {
            let cb = on_pointer.clone();
            move |ev: PointerEvent| Some(cb(i, ev))
        };
        hijos.push(columna_editor(
            ides[i],
            cuerpos[i],
            i == activo,
            palette_editor,
            palette_lienzo,
            cfg,
            metrics,
            visible_lines,
            language,
            on_pointer_i,
        ));
        if i + 1 < ides.len() {
            let carta = cartas.get(i).copied().flatten();
            hijos.push(carril_editor(
                ides[i],
                ides[i + 1],
                carta,
                cfg,
                paleta_hebras,
                metrics,
            ));
        }
    }

    // Con ancho fijo, el HStack mide su ancho real (suma de columnas +
    // carriles) para poder desbordar el viewport — el caller scrollea.
    // Sin ancho fijo, ocupa el 100% y las columnas elásticas se reparten.
    let ancho_root = match cfg.ancho_cuerpo {
        Some(w) => {
            let n = ides.len() as f32;
            length(n * w + (n - 1.0).max(0.0) * cfg.ancho_carril)
        }
        None => percent(1.0_f32),
    };

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: ancho_root,
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(palette_lienzo.bg_app)
    .children(hijos)
}

/// Una columna: wrapper que pinta el borde de foco cuando el cuerpo
/// está activo, header con el nombre del cuerpo arriba, editor real
/// abajo expandido a flex-grow.
#[allow(clippy::too_many_arguments)]
fn columna_editor<Msg, FPtr>(
    ide: &CuerpoIde,
    cuerpo: &Cuerpo,
    activo: bool,
    palette_editor: &EditorPalette,
    palette_lienzo: &Palette,
    cfg: &ConfigMultilienzoEditor,
    metrics: EditorMetrics,
    visible_lines: usize,
    language: Language,
    on_pointer: FPtr,
) -> View<Msg>
where
    Msg: Clone + 'static,
    FPtr: Fn(PointerEvent) -> Option<Msg> + Send + Sync + Clone + 'static,
{
    let header_text = format!(
        "{} · {}",
        cuerpo.metadatos.nombre_legible,
        intencion_label(&cuerpo.metadatos.intencion),
    );
    let header_color = if activo {
        palette_lienzo.border_strong
    } else {
        palette_lienzo.fg_muted
    };
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
    .fill(palette_lienzo.bg_panel)
    .text_aligned(header_text, 11.0, header_color, Alignment::Start);

    let editor = cuerpo_ide_view::<Msg>(
        ide,
        palette_editor,
        metrics,
        visible_lines,
        language,
        on_pointer,
    );
    // Overlay con divisores entre átomos: una línea horizontal sutil en
    // la "línea vacía" del separador (la línea blanca del `\n\n`) entre
    // cada par de átomos consecutivos. Saca el ojo del muro de texto y
    // marca dónde termina cada párrafo lógico. El overlay no tiene
    // handler de click, así que es transparente al hit-test —
    // `paint_with` solo dibuja, no captura.
    let overlay_separadores =
        overlay_separadores_atomos::<Msg>(ide, metrics, palette_lienzo);

    let contenedor_editor = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(palette_editor.bg)
    .children(vec![editor, overlay_separadores]);

    // Wrapper con padding accent cuando es el activo — el padding actúa
    // como borde grueso visible (Llimphi todavía no expone `border()`
    // en View, así que usamos fill + padding para simularlo).
    let pad = if activo { cfg.grosor_foco } else { 0.0 };
    let fondo_wrapper = if activo {
        palette_lienzo.border_strong
    } else {
        palette_lienzo.bg_app
    };
    // Ancho fijo → columna rígida que desborda (scroll del caller). Elástico
    // → flex_grow reparte el viewport entre columnas.
    let (ancho_wrapper, flex_wrapper) = match cfg.ancho_cuerpo {
        Some(w) => (length(w), 0.0),
        None => (percent(1.0_f32), 1.0),
    };
    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: flex_wrapper,
        flex_shrink: 0.0,
        size: Size {
            width: ancho_wrapper,
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(pad),
            right: length(pad),
            top: length(pad),
            bottom: length(pad),
        },
        ..Default::default()
    })
    .fill(fondo_wrapper)
    .children(vec![View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![header, contenedor_editor])])
}

/// Carril entre dos editores: pinta las hebras de la carta correspondiente
/// con `paint_with`. Las posiciones Y se resuelven contra los ides vivos
/// (línea inicial del átomo × `line_height`, menos `scroll_offset`).
fn carril_editor<Msg: Clone + 'static>(
    izq: &CuerpoIde,
    der: &CuerpoIde,
    carta: Option<&CartaHebras>,
    cfg: &ConfigMultilienzoEditor,
    paleta: &PaletaHebras,
    metrics: EditorMetrics,
) -> View<Msg> {
    let hebras = precomputar_hebras_editor(izq, der, carta, cfg, paleta, metrics);
    let nodo = View::new(Style {
        size: Size {
            width: length(cfg.ancho_carril),
            height: percent(1.0_f32),
        },
        ..Default::default()
    });
    if hebras.is_empty() {
        return nodo;
    }
    nodo.paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::peniko::Fill;
        let alto = rect.h;
        // Curva-S con tangentes horizontales en ambos extremos (igual que
        // `pineal-flow`): el control point arranca a `0.5 * ancho` del extremo.
        let dx = (rect.w * 0.5) as f64;
        let x1 = rect.x as f64;
        let x2 = (rect.x + rect.w) as f64;
        for h in &hebras {
            // Clamp de cada borde al alto visible del carril.
            let it = (rect.y + h.izq_top.clamp(0.0, alto)) as f64;
            let ib = (rect.y + h.izq_bot.clamp(0.0, alto)) as f64;
            let dt = (rect.y + h.der_top.clamp(0.0, alto)) as f64;
            let db = (rect.y + h.der_bot.clamp(0.0, alto)) as f64;
            // Cinta cerrada: borde superior (S) → lado derecho → borde
            // inferior (S de vuelta) → lado izquierdo (close) → relleno.
            let mut path = BezPath::new();
            path.move_to((x1, it));
            path.curve_to((x1 + dx, it), (x2 - dx, dt), (x2, dt));
            path.line_to((x2, db));
            path.curve_to((x2 - dx, db), (x1 + dx, ib), (x1, ib));
            path.close_path();
            scene.fill(Fill::NonZero, Affine::IDENTITY, h.color, None, &path);
        }
    })
}

/// Arma el overlay que pinta los divisores entre átomos. Va sobre el
/// editor — superpuesto al contenido del text-editor pero detrás de
/// nada (es el último hijo del contenedor). Sin `on_click`, así que el
/// hit-test del runtime lo ignora y los clicks llegan al editor abajo.
fn overlay_separadores_atomos<Msg: Clone + 'static>(
    ide: &CuerpoIde,
    metrics: EditorMetrics,
    palette_lienzo: &Palette,
) -> View<Msg> {
    let ys = precomputar_y_separadores(ide, metrics);
    let line_h = metrics.line_height;
    let gutter = metrics.gutter_width as f64;
    // Color sutil: fg_muted con alpha reducido — visible pero sin
    // competir con el texto.
    let base = palette_lienzo.fg_muted.components;
    let color = Color::new([base[0], base[1], base[2], base[3] * 0.35]);

    let nodo = View::new(Style {
        position: llimphi_ui::llimphi_layout::taffy::Position::Absolute,
        inset: Rect {
            left: length(0.0_f32),
            top: length(0.0_f32),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    });
    if ys.is_empty() {
        return nodo;
    }
    nodo.paint_with(move |scene, _ts, rect| {
        let stroke = Stroke::new(1.0);
        for &y_local in &ys {
            // El editor no tiene padding vertical interno; rangos válidos
            // del overlay son [0, rect.h]. Las líneas fuera de viewport
            // se omiten (no se pintan recortadas — confundiría).
            if y_local < 0.0 || y_local > rect.h {
                continue;
            }
            // Salta el gutter (no separamos sobre los números de línea).
            let x1 = rect.x as f64 + gutter;
            let x2 = (rect.x + rect.w) as f64;
            let y = (rect.y + y_local) as f64;
            let mut path = BezPath::new();
            path.move_to((x1, y));
            path.line_to((x2, y));
            scene.stroke(&stroke, Affine::IDENTITY, color, None, &path);
        }
        // suppress unused warning if compiler complains about line_h
        let _ = line_h;
    })
}

/// Devuelve los Y locales (en el rect del editor, sin contar el header
/// que vive en otro nodo) donde cae el separador entre átomos
/// consecutivos del ide, ajustados al scroll actual.
fn precomputar_y_separadores(ide: &CuerpoIde, metrics: EditorMetrics) -> Vec<f32> {
    let mut out = Vec::new();
    if ide.editor_cuerpo.atom_ids.len() < 2 {
        return out;
    }
    let scroll = ide.state.scroll_offset as f32;
    for i in 1..ide.editor_cuerpo.atom_ids.len() {
        let id = ide.editor_cuerpo.atom_ids[i];
        let Some((line, _)) = ide.posicion_de_atom(id) else {
            continue;
        };
        // El SEPARADOR es `\n\n`, que aporta una línea vacía entre dos
        // párrafos. El átomo `i` arranca en `line`; la línea vacía es
        // `line - 1`. Si el átomo arranca en 0 (no debería pasar para
        // i >= 1), saltamos.
        if line == 0 {
            continue;
        }
        let linea_sep = (line - 1) as f32;
        let y = (linea_sep - scroll + 0.5) * metrics.line_height;
        out.push(y);
    }
    out
}

/// Construye las **cintas Sankey** entre dos columnas. Empareja secciones por
/// la carta de hebras si la hay (respeta su alineamiento real, en cualquier
/// orden atom_a/atom_b); si no hay carta (o está vacía), cae a un emparejado
/// **posicional** (sección *i* ↔ sección *i*) — así las cintas fluyen por
/// todos los lienzos aunque no exista una carta entre ese par puntual, que es
/// el caso típico de traducciones paralelas.
///
/// El grosor de la cinta es la altura de la sección en cada lado (no un punto):
/// arranca y termina cubriendo el bloque de párrafo completo.
fn precomputar_hebras_editor(
    izq: &CuerpoIde,
    der: &CuerpoIde,
    carta: Option<&CartaHebras>,
    cfg: &ConfigMultilienzoEditor,
    paleta: &PaletaHebras,
    metrics: EditorMetrics,
) -> Vec<HebraEditor> {
    let header = cfg.alto_header;
    let lh = metrics.line_height;
    // Hueco vertical entre cintas vecinas, para que se lean separadas.
    const GAP: f32 = 2.5;

    // Rango vertical [top, bot] que ocupa la sección `idx` de un ide, en
    // coords locales al carril (con header + scroll).
    let extent = |ide: &CuerpoIde, idx: usize| -> Option<(f32, f32)> {
        let ids = &ide.editor_cuerpo.atom_ids;
        let id = *ids.get(idx)?;
        let (start, _) = ide.posicion_de_atom(id)?;
        let end = if idx + 1 < ids.len() {
            ide.posicion_de_atom(ids[idx + 1])
                .map(|(l, _)| l.saturating_sub(1))
                .unwrap_or(start + 1)
        } else {
            ide.state.line_count()
        };
        let scroll = ide.state.scroll_offset as f32;
        let top = header + (start as f32 - scroll) * lh + GAP;
        let bot = header + (end as f32 - scroll) * lh - GAP;
        Some((top, bot.max(top + 1.0)))
    };
    let idx_of =
        |ide: &CuerpoIde, id: Uuid| ide.editor_cuerpo.atom_ids.iter().position(|x| *x == id);

    // Pares (idx_izq, idx_der, color) a unir.
    let mut pares: Vec<(usize, usize, Color)> = Vec::new();
    let con_carta = carta.map(|c| !c.hebras.is_empty()).unwrap_or(false);
    if let (true, Some(c)) = (con_carta, carta) {
        for h in &c.hebras {
            let (ii, jj) = if let (Some(a), Some(b)) = (idx_of(izq, h.atom_a), idx_of(der, h.atom_b))
            {
                (a, b)
            } else if let (Some(a), Some(b)) = (idx_of(izq, h.atom_b), idx_of(der, h.atom_a)) {
                (a, b)
            } else {
                continue;
            };
            pares.push((ii, jj, color_hebra(cfg, ii, paleta, Some(h))));
        }
    } else {
        // Emparejado posicional: la sección i fluye a la sección i.
        let n = izq
            .editor_cuerpo
            .atom_ids
            .len()
            .min(der.editor_cuerpo.atom_ids.len());
        for i in 0..n {
            pares.push((i, i, color_hebra(cfg, i, paleta, None)));
        }
    }

    let mut out = Vec::with_capacity(pares.len());
    for (ii, jj, color) in pares {
        let (Some((it, ib)), Some((dt, db))) = (extent(izq, ii), extent(der, jj)) else {
            continue;
        };
        out.push(HebraEditor {
            izq_top: it,
            izq_bot: ib,
            der_top: dt,
            der_bot: db,
            color,
        });
    }
    out
}

/// Color de una cinta. Con `colorear_secciones` usa el color de identidad de
/// la sección (mismo en todos los lienzos → flujo continuo de un color);
/// si no, el color por origen del alineamiento. Translúcido para que los
/// solapes se lean; stale (carta no fresca) más tenue.
fn color_hebra(
    cfg: &ConfigMultilienzoEditor,
    idx_izq: usize,
    paleta: &PaletaHebras,
    hebra: Option<&pluma_align::Alineamiento>,
) -> Color {
    const ALPHA: f32 = 0.5;
    const ALPHA_STALE: f32 = 0.22;
    let fresco = hebra.map(|h| h.fresco).unwrap_or(true);
    let alpha = if fresco { ALPHA } else { ALPHA_STALE };
    let base = if cfg.colorear_secciones {
        color_seccion(idx_izq)
    } else if let Some(h) = hebra {
        if !h.fresco {
            paleta.stale
        } else {
            match &h.origen {
                OrigenAlineamiento::Derivado { .. } => paleta.derivada,
                OrigenAlineamiento::Manual { .. } => paleta.manual,
                OrigenAlineamiento::Embeddings { .. } => paleta.embeddings,
            }
        }
    } else {
        color_seccion(idx_izq)
    };
    let [r, g, b, _] = base.components;
    Color::new([r, g, b, alpha])
}

/// Copia el `scroll_offset` del cuerpo activo al resto de los editores —
/// el patrón estándar para mantener las hebras alineadas cuando el
/// usuario scrollea uno solo. Cada destino clampea al fin de su buffer
/// (si el cuerpo destino es más corto, su scroll queda topado en su
/// última línea — el viewport muestra menos contenido, pero nunca
/// líneas espurias arriba).
///
/// El caller suele llamar esto al final de cada `update` que pueda
/// haber tocado el scroll del activo (typing con `ensure_caret_visible`,
/// PageUp/PageDown, click+set_caret).
pub fn sincronizar_scroll_desde_activo(ides: &mut [CuerpoIde], activo: usize) {
    if activo >= ides.len() {
        return;
    }
    let scroll = ides[activo].state.scroll_offset;
    sincronizar_scroll(ides, scroll, activo);
}

/// Versión explícita: aplica `scroll` a todos los `ides` salvo el índice
/// `excepto`. Útil cuando el caller ya tiene el valor de scroll (p.ej.
/// porque viene de un wheel event futuro) y no quiere depender del
/// estado del activo.
pub fn sincronizar_scroll(ides: &mut [CuerpoIde], scroll: usize, excepto: usize) {
    for (i, ide) in ides.iter_mut().enumerate() {
        if i == excepto {
            continue;
        }
        let max = ide.state.line_count().saturating_sub(1);
        ide.state.scroll_offset = scroll.min(max);
    }
}

/// Rótulo corto y legible para cada variante de `Intencion`. Copiado
/// (no factorizado) de `multilienzo.rs`: son dos vistas distintas con
/// dos paletas distintas, conviene que cada una controle su rótulo.
fn intencion_label(intencion: &pluma_cuerpo::Intencion) -> String {
    use pluma_cuerpo::Intencion;
    match intencion {
        Intencion::Original => "original".to_string(),
        Intencion::Traduccion => "traducción".to_string(),
        Intencion::Tono { etiqueta } => format!("tono: {etiqueta}"),
        Intencion::Resumen {
            palabras_objetivo: Some(n),
        } => format!("resumen ≈{n}p"),
        Intencion::Resumen {
            palabras_objetivo: None,
        } => "resumen".to_string(),
        Intencion::Reescritura { .. } => "reescritura".to_string(),
        Intencion::Anotacion => "anotación".to_string(),
        Intencion::Custom { kind } => kind.clone(),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use pluma_align::{alinear_uno_a_uno, OrigenAlineamiento};
    use pluma_core::NarrativeAtom;
    use pluma_cuerpo::{Cuerpo, Intencion};
    use std::collections::HashMap;

    fn ide_con_textos(branch: &str, intencion: Intencion, textos: &[&str]) -> (Cuerpo, Vec<NarrativeAtom>, CuerpoIde) {
        let mut c = Cuerpo::nuevo(branch, branch, intencion, 100);
        let atoms: Vec<NarrativeAtom> = textos
            .iter()
            .map(|t| NarrativeAtom::new(*t, branch))
            .collect();
        for a in &atoms {
            c.agregar(a.id, 101);
        }
        let idx: HashMap<Uuid, &NarrativeAtom> = atoms.iter().map(|a| (a.id, a)).collect();
        let ide = CuerpoIde::from_cuerpo(&c, &idx);
        (c, atoms, ide)
    }

    #[test]
    fn separadores_se_computan_uno_por_juntura_entre_atomos() {
        // 3 átomos → 2 separadores. El primer átomo arranca en línea 0,
        // los siguientes en 2 y 4 (con SEPARADOR `\n\n` = 1 línea vacía
        // entre cada par).
        let (_, _, ide) = ide_con_textos("es", Intencion::Original, &["uno", "dos", "tres"]);
        let metrics = EditorMetrics::for_font_size(13.0);
        let ys = precomputar_y_separadores(&ide, metrics);
        assert_eq!(ys.len(), 2);
        // Separador entre átomo 0 y 1 → línea 1 (atomo[1] arranca en 2).
        let y_sep_01 = (1.0 + 0.5) * metrics.line_height;
        assert!((ys[0] - y_sep_01).abs() < 1e-3);
        // Separador entre átomo 1 y 2 → línea 3 (atomo[2] arranca en 4).
        let y_sep_12 = (3.0 + 0.5) * metrics.line_height;
        assert!((ys[1] - y_sep_12).abs() < 1e-3);
    }

    #[test]
    fn separadores_siguen_al_scroll() {
        let (_, _, mut ide) = ide_con_textos("es", Intencion::Original, &["uno", "dos"]);
        let metrics = EditorMetrics::for_font_size(13.0);
        let antes = precomputar_y_separadores(&ide, metrics);
        ide.state.scroll_offset = 2;
        let despues = precomputar_y_separadores(&ide, metrics);
        // El separador queda más arriba cuando se scrollea hacia abajo:
        // la diferencia debe ser exactamente 2 × line_height.
        assert_eq!(antes.len(), 1);
        assert_eq!(despues.len(), 1);
        let delta = antes[0] - despues[0];
        assert!((delta - 2.0 * metrics.line_height).abs() < 1e-3);
    }

    #[test]
    fn separadores_vacio_para_un_solo_atomo() {
        let (_, _, ide) = ide_con_textos("es", Intencion::Original, &["uno"]);
        let metrics = EditorMetrics::for_font_size(13.0);
        let ys = precomputar_y_separadores(&ide, metrics);
        assert!(ys.is_empty());
    }

    #[test]
    fn vacio_devuelve_vista_sin_panico() {
        let v: View<()> = multilienzo_editor_view(
            &[],
            &[],
            &[],
            0,
            &EditorPalette::default(),
            &PaletaHebras::default(),
            &Palette::default(),
            &ConfigMultilienzoEditor::default(),
            EditorMetrics::for_font_size(13.0),
            100,
            Language::Plain,
            |_, _| (),
        );
        let _ = v;
    }

    #[test]
    fn precomputar_hebras_alinea_centros_de_linea_con_scroll() {
        let (a, _atoms_a, ide_a) = ide_con_textos("es", Intencion::Original, &["uno", "dos"]);
        let (b, _atoms_b, ide_b) = ide_con_textos("qu", Intencion::Traduccion, &["huk", "iskay"]);
        let carta = alinear_uno_a_uno(
            &a,
            &b,
            OrigenAlineamiento::Derivado {
                transformacion: Uuid::new_v4(),
                timestamp: 1,
            },
        );
        let cfg = ConfigMultilienzoEditor::default();
        let paleta = PaletaHebras::default();
        let metrics = EditorMetrics::for_font_size(13.0);

        let hebras =
            precomputar_hebras_editor(&ide_a, &ide_b, Some(&carta), &cfg, &paleta, metrics);
        assert_eq!(hebras.len(), 2);

        // La cinta de una sección es simétrica: el rango vertical izquierdo
        // coincide con el derecho (traducción 1-1, mismas líneas).
        assert!((hebras[0].izq_top - hebras[0].der_top).abs() < 1e-3);
        assert!((hebras[0].izq_bot - hebras[0].der_bot).abs() < 1e-3);

        // La sección 1 arranca 2 líneas (contenido + separador) debajo de la 0.
        let salto = hebras[1].izq_top - hebras[0].izq_top;
        assert!((salto - 2.0 * metrics.line_height).abs() < 1e-3);
    }

    #[test]
    fn stale_baja_el_alpha_de_la_cinta() {
        let (a, _atoms_a, ide_a) = ide_con_textos("es", Intencion::Original, &["x"]);
        let (b, _atoms_b, ide_b) = ide_con_textos("qu", Intencion::Traduccion, &["y"]);
        let mut carta = alinear_uno_a_uno(
            &a,
            &b,
            OrigenAlineamiento::Embeddings {
                modelo: "iniy-1".into(),
                timestamp: 100,
            },
        );
        carta.hebras[0].fresco = false;

        let hebras = precomputar_hebras_editor(
            &ide_a,
            &ide_b,
            Some(&carta),
            &ConfigMultilienzoEditor::default(),
            &PaletaHebras::default(),
            EditorMetrics::for_font_size(13.0),
        );
        assert_eq!(hebras.len(), 1);
        // Cinta stale = más tenue (alpha bajo).
        assert!(hebras[0].color.components[3] < 0.3);
    }

    #[test]
    fn sin_carta_empareja_por_posicion() {
        // Sin carta entre el par, las cintas igual fluyen sección-a-sección.
        let (_, _, ide_a) = ide_con_textos("es", Intencion::Original, &["uno", "dos", "tres"]);
        let (_, _, ide_b) = ide_con_textos("qu", Intencion::Traduccion, &["huk", "iskay"]);
        let hebras = precomputar_hebras_editor(
            &ide_a,
            &ide_b,
            None,
            &ConfigMultilienzoEditor::default(),
            &PaletaHebras::default(),
            EditorMetrics::for_font_size(13.0),
        );
        // min(3, 2) = 2 cintas, por posición.
        assert_eq!(hebras.len(), 2);
    }

    #[test]
    fn sincronizar_scroll_copia_al_resto_y_clampea() {
        // Cuerpo activo largo: 10 átomos. Cuerpos destino: uno largo, uno corto.
        let textos_largos: Vec<String> = (0..10).map(|i| format!("p{i}")).collect();
        let textos_largos_ref: Vec<&str> = textos_largos.iter().map(|s| s.as_str()).collect();
        let (_, _, ide_largo_a) = ide_con_textos("es", Intencion::Original, &textos_largos_ref);
        let (_, _, ide_largo_b) = ide_con_textos("qu", Intencion::Traduccion, &textos_largos_ref);
        let (_, _, ide_corto) = ide_con_textos("en", Intencion::Traduccion, &["solo uno"]);

        let mut ides = vec![ide_largo_a, ide_largo_b, ide_corto];
        ides[0].state.scroll_offset = 12; // activo scrollea hacia abajo

        sincronizar_scroll_desde_activo(&mut ides, 0);

        // El otro cuerpo largo recibe el scroll tal cual (su line_count
        // permite scrollear más allá de 12).
        assert!(ides[1].state.scroll_offset >= 12 - 1);
        // El cuerpo corto se clampea a su última línea (solo tiene 1
        // párrafo ⇒ line_count == 1 ⇒ max_scroll == 0).
        assert_eq!(ides[2].state.scroll_offset, 0);
        // El activo no se toca.
        assert_eq!(ides[0].state.scroll_offset, 12);
    }

    #[test]
    fn sincronizar_scroll_es_idempotente_sin_cambios() {
        let (_, _, mut ide_a) = ide_con_textos("es", Intencion::Original, &["uno", "dos", "tres"]);
        let (_, _, mut ide_b) = ide_con_textos("qu", Intencion::Traduccion, &["huk", "iskay", "kimsa"]);
        ide_a.state.scroll_offset = 2;
        ide_b.state.scroll_offset = 2;
        let mut ides = vec![ide_a, ide_b];

        sincronizar_scroll_desde_activo(&mut ides, 0);
        sincronizar_scroll_desde_activo(&mut ides, 0);
        assert_eq!(ides[0].state.scroll_offset, 2);
        assert_eq!(ides[1].state.scroll_offset, 2);
    }

    #[test]
    fn scroll_offset_desplaza_y_de_la_hebra() {
        let (a, _atoms_a, mut ide_a) = ide_con_textos("es", Intencion::Original, &["uno"]);
        let (b, _atoms_b, ide_b) = ide_con_textos("qu", Intencion::Traduccion, &["huk"]);
        let carta = alinear_uno_a_uno(
            &a,
            &b,
            OrigenAlineamiento::Derivado {
                transformacion: Uuid::new_v4(),
                timestamp: 1,
            },
        );
        let cfg = ConfigMultilienzoEditor::default();
        let paleta = PaletaHebras::default();
        let metrics = EditorMetrics::for_font_size(13.0);

        let antes = precomputar_hebras_editor(&ide_a, &ide_b, Some(&carta), &cfg, &paleta, metrics);
        ide_a.state.scroll_offset = 3;
        let despues =
            precomputar_hebras_editor(&ide_a, &ide_b, Some(&carta), &cfg, &paleta, metrics);
        // El lado izquierdo se desplaza 3 líneas hacia arriba; el lado
        // derecho queda igual.
        let delta = antes[0].izq_top - despues[0].izq_top;
        assert!((delta - 3.0 * metrics.line_height).abs() < 1e-3);
        assert!((antes[0].der_top - despues[0].der_top).abs() < 1e-3);
    }
}
