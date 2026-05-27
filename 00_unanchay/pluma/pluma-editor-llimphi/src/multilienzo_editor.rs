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
}

impl Default for ConfigMultilienzoEditor {
    fn default() -> Self {
        Self {
            ancho_carril: 56.0,
            alto_header: 28.0,
            grosor_hebra: 2.0,
            grosor_foco: 2.0,
        }
    }
}

/// Datos pre-calculados de una hebra entre dos editores vivos.
#[derive(Debug, Clone, Copy)]
struct HebraEditor {
    /// Y en píxeles dentro del rect del carril (ya considera el
    /// `scroll_offset` y `alto_header` de cada editor).
    y_izq: f32,
    y_der: f32,
    color: Color,
    punteada: bool,
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

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
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
    let contenedor_editor = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(palette_editor.bg)
    .children(vec![editor]);

    // Wrapper con padding accent cuando es el activo — el padding actúa
    // como borde grueso visible (Llimphi todavía no expone `border()`
    // en View, así que usamos fill + padding para simularlo).
    let pad = if activo { cfg.grosor_foco } else { 0.0 };
    let fondo_wrapper = if activo {
        palette_lienzo.border_strong
    } else {
        palette_lienzo.bg_app
    };
    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
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
    let hebras = match carta {
        Some(c) => precomputar_hebras_editor(izq, der, c, cfg, paleta, metrics),
        None => Vec::new(),
    };
    let grosor = cfg.grosor_hebra;
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
        let solido = Stroke::new(grosor as f64);
        let punteado = Stroke::new(grosor as f64).with_dashes(0.0, [6.0, 4.0]);
        let alto_carril = rect.h;
        for h in &hebras {
            // Clamp suave al alto del carril — cuando un átomo está fuera
            // del viewport, la hebra se "asoma" pegada al borde.
            let y_izq = h.y_izq.clamp(0.0, alto_carril);
            let y_der = h.y_der.clamp(0.0, alto_carril);
            let mut path = BezPath::new();
            path.move_to((rect.x as f64, (rect.y + y_izq) as f64));
            path.line_to(((rect.x + rect.w) as f64, (rect.y + y_der) as f64));
            let stroke = if h.punteada { &punteado } else { &solido };
            scene.stroke(stroke, Affine::IDENTITY, h.color, None, &path);
        }
    })
}

/// Resuelve para cada hebra de la carta su posición Y en cada editor.
/// Acepta que la carta tenga `atom_a/atom_b` en cualquier orden respecto
/// a `izq/der` — ya lo hacía el multilienzo readonly, replicamos la
/// misma robustez acá.
fn precomputar_hebras_editor(
    izq: &CuerpoIde,
    der: &CuerpoIde,
    carta: &CartaHebras,
    cfg: &ConfigMultilienzoEditor,
    paleta: &PaletaHebras,
    metrics: EditorMetrics,
) -> Vec<HebraEditor> {
    let header = cfg.alto_header;
    let y_de_atom = |ide: &CuerpoIde, id: Uuid| -> Option<f32> {
        let (line, _) = ide.posicion_de_atom(id)?;
        let scroll = ide.state.scroll_offset as f32;
        // Centro vertical de la línea, en coordenadas locales al carril.
        Some(header + (line as f32 - scroll + 0.5) * metrics.line_height)
    };

    let mut out = Vec::with_capacity(carta.hebras.len());
    for h in &carta.hebras {
        let (y_izq, y_der) = if let (Some(a), Some(b)) =
            (y_de_atom(izq, h.atom_a), y_de_atom(der, h.atom_b))
        {
            (a, b)
        } else if let (Some(a), Some(b)) =
            (y_de_atom(izq, h.atom_b), y_de_atom(der, h.atom_a))
        {
            (a, b)
        } else {
            continue;
        };

        let (color_base, modular_fuerza) = if !h.fresco {
            (paleta.stale, false)
        } else {
            match &h.origen {
                OrigenAlineamiento::Derivado { .. } => (paleta.derivada, false),
                OrigenAlineamiento::Manual { .. } => (paleta.manual, false),
                OrigenAlineamiento::Embeddings { .. } => (paleta.embeddings, true),
            }
        };
        let color = if modular_fuerza {
            modular_alpha(color_base, h.fuerza)
        } else {
            color_base
        };

        out.push(HebraEditor {
            y_izq,
            y_der,
            color,
            punteada: !h.fresco,
        });
    }
    out
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

fn modular_alpha(c: Color, factor: f32) -> Color {
    let f = factor.clamp(0.0, 1.0);
    let [r, g, b, a] = c.components;
    Color::new([r, g, b, a * f])
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

        let hebras = precomputar_hebras_editor(&ide_a, &ide_b, &carta, &cfg, &paleta, metrics);
        assert_eq!(hebras.len(), 2);

        // El primer átomo arranca en línea 0 — centro vertical = header + 0.5 * line_height.
        let y_esperada_atom_0 = cfg.alto_header + 0.5 * metrics.line_height;
        assert!((hebras[0].y_izq - y_esperada_atom_0).abs() < 1e-3);
        assert!((hebras[0].y_der - y_esperada_atom_0).abs() < 1e-3);

        // El segundo átomo arranca después del primer párrafo (1 línea de
        // contenido + 1 línea vacía del separador) = línea 2.
        let y_esperada_atom_1 = cfg.alto_header + (2.0 + 0.5) * metrics.line_height;
        assert!((hebras[1].y_izq - y_esperada_atom_1).abs() < 1e-3);
    }

    #[test]
    fn stale_pinta_punteada() {
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
            &carta,
            &ConfigMultilienzoEditor::default(),
            &PaletaHebras::default(),
            EditorMetrics::for_font_size(13.0),
        );
        assert_eq!(hebras.len(), 1);
        assert!(hebras[0].punteada);
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

        let antes = precomputar_hebras_editor(&ide_a, &ide_b, &carta, &cfg, &paleta, metrics);
        ide_a.state.scroll_offset = 3;
        let despues = precomputar_hebras_editor(&ide_a, &ide_b, &carta, &cfg, &paleta, metrics);
        // El lado izquierdo se desplaza 3 líneas hacia arriba; el lado
        // derecho queda igual.
        let delta = antes[0].y_izq - despues[0].y_izq;
        assert!((delta - 3.0 * metrics.line_height).abs() < 1e-3);
        assert!((antes[0].y_der - despues[0].y_der).abs() < 1e-3);
    }
}
