//! `lienzos` — render jerárquico: el outline de un cuerpo como **lienzos
//! anidados**.
//!
//! Donde [`crate::multilienzo_editor`] pinta cada cuerpo como un único
//! text-editor plano de fuente uniforme, este módulo lo pinta como un árbol de
//! cajas: cada título (`#`, `##`, …) es un **lienzo que contiene su contenido**
//! —los párrafos que le siguen y las subsecciones más profundas— y cada nivel
//! lleva su propio tamaño de fuente (h1 > h2 > h3 > … > cuerpo). La jerarquía la
//! calcula `pluma-outline` desde la lista plana de átomos; este crate solo la
//! dibuja.
//!
//! Es la superficie del modo **Editar** de la app unificada (la otra mitad,
//! Presentar, vuela el mismo árbol como marcos espaciales — ver el deck).
//!
//! Cada caja (título o párrafo) es clickeable: emite `on_select(atom_id)` para
//! que el caller marque ese átomo como objetivo de edición in-situ. El render en
//! sí es puro; no posee estado de edición.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::prelude::{
    auto, length, percent, FlexDirection, Position, Rect, Size, Style,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_widget_text_editor::{
    text_editor_view, EditorMetrics, EditorPalette, PointerEvent,
};
use pluma_align::CartaHebras;
use pluma_core::NarrativeAtom;
use pluma_cuerpo::Cuerpo;
use pluma_outline::{font_size_por_nivel, proyectar, Nodo, Seccion};
use uuid::Uuid;

use crate::multilienzo_editor::color_seccion;
use crate::Palette;

thread_local! {
    /// Posición ABSOLUTA (x, y, w, h) en coords de escena donde se pintó la caja
    /// de cada átomo este frame. La llenan las cajas al pintarse (paint_with) y
    /// la lee el overlay de hebras (que pinta último) para tender las cintas
    /// entre columnas, igual que `panel_actual` del deck. Se limpia tras leerla.
    static REGISTRO: RefCell<HashMap<Uuid, (f32, f32, f32, f32)>> =
        RefCell::new(HashMap::new());
}

/// Agrega a una caja un `paint_with` que registra su rect absoluto bajo `atom`,
/// sin dibujar nada (el texto se pinta encima). Permite que el overlay de hebras
/// sepa dónde quedó cada átomo en el layout flow.
fn registrar_atom<Msg: Clone + 'static>(v: View<Msg>, atom: Uuid) -> View<Msg> {
    v.paint_with(move |_scene, _ts, rect| {
        REGISTRO.with(|r| {
            r.borrow_mut()
                .insert(atom, (rect.x, rect.y, rect.w, rect.h));
        });
    })
}

/// Líneas máximas que dibuja el editor in-situ de un átomo (cap del viewport).
const VISIBLE_INLINE: usize = 80;

/// Contexto de ejecución de celdas (modo notebook embebido): un lienzo cuyo
/// átomo es una celda ` ```llm … ``` ` muestra un botón ▶ y su última salida.
pub struct EjecucionLienzo<'a, Msg> {
    /// Salida por átomo-celda (texto), si ya se ejecutó.
    pub salidas: &'a HashMap<Uuid, String>,
    /// Click en ▶ ejecutar → Msg (el caller corre la celda).
    pub on_run: Arc<dyn Fn(Uuid) -> Msg + Send + Sync>,
}

/// Si `texto` es una celda LLM (fence ` ```llm `), devuelve el cuerpo (el
/// prompt). El átomo lo escribe el usuario literalmente en el editor in-situ.
pub fn celda_llm(texto: &str) -> Option<String> {
    let t = texto.trim_start();
    let rest = t.strip_prefix("```")?;
    let mut lineas = rest.lines();
    let lang = lineas.next().unwrap_or("").trim();
    if !lang.eq_ignore_ascii_case("llm") {
        return None;
    }
    let mut body = String::new();
    for l in lineas {
        if l.trim_start().starts_with("```") {
            break;
        }
        body.push_str(l);
        body.push('\n');
    }
    Some(body.trim().to_string())
}

/// Contexto de edición in-situ: qué átomo se está editando, con qué estado de
/// editor, y a dónde mandar sus eventos de puntero. Cuando el render topa con
/// `atom`, en vez del texto estático pinta el widget text-editor cargado con
/// `state` (a la fuente del nivel del átomo: un `#` se edita grande).
pub struct EdicionLienzo<'a, Msg> {
    /// Átomo en edición.
    pub atom: Uuid,
    /// Estado del editor (buffer + caret + undo) del átomo en edición.
    pub state: &'a llimphi_widget_text_editor::EditorState,
    /// Paleta del text-editor.
    pub palette: &'a EditorPalette,
    /// Click/drag dentro del editor in-situ → Msg (el caller mueve el caret).
    pub on_pointer: Arc<dyn Fn(PointerEvent) -> Msg + Send + Sync>,
}

/// Geometría del render de lienzos.
#[derive(Debug, Clone, Copy)]
pub struct ConfigLienzos {
    /// Tamaño de fuente del cuerpo (párrafos). Los títulos escalan sobre este
    /// con [`font_size_por_nivel`].
    pub font_base: f32,
    /// Padding interior de cada lienzo (sección), en px. Da también el sangrado
    /// visual del anidamiento.
    pub padding: f32,
    /// Hueco vertical entre nodos hermanos, en px.
    pub gap: f32,
    /// Ancho de cada columna de cuerpo. `None` = elástica (se reparte el
    /// viewport); `Some(w)` = fija (desborda y el caller scrollea).
    pub ancho_cuerpo: Option<f32>,
}

impl Default for ConfigLienzos {
    fn default() -> Self {
        Self {
            font_base: 15.0,
            padding: 10.0,
            gap: 8.0,
            ancho_cuerpo: None,
        }
    }
}

/// Render de **un** cuerpo como lienzos anidados. `atoms` resuelve el texto de
/// cada átomo. `seleccionado` recibe un realce (el átomo en edición). Cada caja
/// emite `on_select(atom)` al click.
pub fn lienzos_cuerpo_view<Msg, FSel>(
    cuerpo: &Cuerpo,
    atoms: &HashMap<Uuid, &NarrativeAtom>,
    palette: &Palette,
    cfg: &ConfigLienzos,
    seleccionado: Option<Uuid>,
    edicion: Option<&EdicionLienzo<Msg>>,
    ejecucion: Option<&EjecucionLienzo<Msg>>,
    on_select: FSel,
) -> View<Msg>
where
    Msg: Clone + 'static,
    FSel: Fn(Uuid) -> Msg + Clone + 'static,
{
    let outline = proyectar(&cuerpo.orden, |id| {
        atoms.get(&id).map(|a| a.content.as_str())
    });

    let hijos = render_nodos(
        &outline.raiz,
        atoms,
        palette,
        cfg,
        seleccionado,
        edicion,
        ejecucion,
        &on_select,
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(cfg.gap),
        },
        padding: Rect {
            left: length(cfg.padding),
            right: length(cfg.padding),
            top: length(cfg.padding),
            bottom: length(cfg.padding),
        },
        ..Default::default()
    })
    .fill(palette.bg_app)
    .children(hijos)
}

/// Render de N cuerpos lado a lado (el multilienzo jerárquico). Cada columna
/// lleva el rótulo del cuerpo arriba y su árbol de lienzos abajo. `activo`
/// marca la columna con foco. `cartas[i]` es la carta de hebras entre la columna
/// `i` y la `i+1` (`None` = sin carta → emparejado posicional): un overlay tiende
/// las **cintas Sankey** entre los átomos alineados, leyendo dónde quedó pintada
/// cada caja (registro por `paint_with`).
#[allow(clippy::too_many_arguments)]
pub fn lienzos_multi_view<Msg, FSel>(
    cuerpos: &[&Cuerpo],
    atoms: &HashMap<Uuid, &NarrativeAtom>,
    palette: &Palette,
    cfg: &ConfigLienzos,
    activo: usize,
    seleccionado: Option<Uuid>,
    edicion: Option<&EdicionLienzo<Msg>>,
    ejecucion: Option<&EjecucionLienzo<Msg>>,
    cartas: &[Option<&CartaHebras>],
    on_select: FSel,
) -> View<Msg>
where
    Msg: Clone + 'static,
    FSel: Fn(Uuid) -> Msg + Clone + 'static,
{
    if cuerpos.is_empty() {
        return View::new(Style::default());
    }
    let mut columnas: Vec<View<Msg>> = Vec::with_capacity(cuerpos.len());
    for (i, cuerpo) in cuerpos.iter().enumerate() {
        let header_color = if i == activo {
            palette.border_strong
        } else {
            palette.fg_muted
        };
        let header = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(28.0_f32),
            },
            padding: Rect {
                left: length(10.0_f32),
                right: length(10.0_f32),
                top: length(7.0_f32),
                bottom: length(7.0_f32),
            },
            ..Default::default()
        })
        .fill(palette.bg_panel)
        .text_aligned(
            cuerpo.metadatos.nombre_legible.clone(),
            11.0,
            header_color,
            Alignment::Start,
        );

        let cuerpo_view = lienzos_cuerpo_view(
            cuerpo,
            atoms,
            palette,
            cfg,
            seleccionado,
            edicion,
            ejecucion,
            on_select.clone(),
        );

        let (ancho, flex) = match cfg.ancho_cuerpo {
            Some(w) => (length(w), 0.0),
            None => (length(0.0_f32), 1.0),
        };
        let columna = View::new(Style {
            flex_direction: FlexDirection::Column,
            flex_grow: flex,
            flex_shrink: 0.0,
            size: Size {
                width: ancho,
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .children(vec![header, cuerpo_view]);
        columnas.push(columna);
    }

    let ancho_root = match cfg.ancho_cuerpo {
        Some(w) => length(cuerpos.len() as f32 * w),
        None => percent(1.0_f32),
    };
    let fila = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        gap: Size {
            width: length(cfg.gap),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_app)
    .children(columnas);

    // Overlay de hebras: pinta DESPUÉS de las columnas (lee el registro de
    // posiciones que ellas dejaron) y tiende las cintas entre átomos alineados.
    let pares = pares_hebras(cuerpos, cartas);
    let overlay = hebras_overlay::<Msg>(pares);

    View::new(Style {
        position: Position::Relative,
        size: Size {
            width: ancho_root,
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![fila, overlay])
}

/// Pares de átomos alineados a unir con cinta entre columnas consecutivas, con
/// su color. Usa la carta de hebras si la hay (en cualquier orden a/b); si no,
/// empareja por posición (átomo `k` ↔ átomo `k`). Color = `color_seccion(k)`.
fn pares_hebras(
    cuerpos: &[&Cuerpo],
    cartas: &[Option<&CartaHebras>],
) -> Vec<(Uuid, Uuid, [f32; 4])> {
    let mut out = Vec::new();
    for i in 0..cuerpos.len().saturating_sub(1) {
        let izq = cuerpos[i];
        let der = cuerpos[i + 1];
        let carta = cartas.get(i).copied().flatten();
        let con_carta = carta.map(|c| !c.hebras.is_empty()).unwrap_or(false);
        if let (true, Some(c)) = (con_carta, carta) {
            for h in &c.hebras {
                let (a, b) = if izq.orden.contains(&h.atom_a) && der.orden.contains(&h.atom_b) {
                    (h.atom_a, h.atom_b)
                } else if izq.orden.contains(&h.atom_b) && der.orden.contains(&h.atom_a) {
                    (h.atom_b, h.atom_a)
                } else {
                    continue;
                };
                let idx = izq.orden.iter().position(|x| *x == a).unwrap_or(0);
                out.push((a, b, color_seccion(idx).components));
            }
        } else {
            let n = izq.orden.len().min(der.orden.len());
            for k in 0..n {
                out.push((izq.orden[k], der.orden[k], color_seccion(k).components));
            }
        }
    }
    out
}

/// Nodo overlay (absoluto, cubre el multilienzo) que pinta las cintas Sankey
/// leyendo el registro de posiciones de los átomos y lo limpia tras leer.
fn hebras_overlay<Msg: Clone + 'static>(pares: Vec<(Uuid, Uuid, [f32; 4])>) -> View<Msg> {
    let nodo = View::new(Style {
        position: Position::Absolute,
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
    if pares.is_empty() {
        return nodo;
    }
    nodo.paint_with(move |scene, _ts, _rect| {
        REGISTRO.with(|reg| {
            let reg = reg.borrow();
            for (a, b, col) in &pares {
                let (Some(&(ax, ay, aw, ah)), Some(&(bx, by, _bw, bh))) =
                    (reg.get(a), reg.get(b))
                else {
                    continue;
                };
                // Borde derecho del átomo izquierdo → borde izquierdo del derecho.
                let x1 = (ax + aw) as f64;
                let x2 = bx as f64;
                if x2 <= x1 {
                    continue;
                }
                let it = ay as f64;
                let ib = (ay + ah) as f64;
                let dt = by as f64;
                let db = (by + bh) as f64;
                let dx = (x2 - x1) * 0.5;
                let mut path = BezPath::new();
                path.move_to((x1, it));
                path.curve_to((x1 + dx, it), (x2 - dx, dt), (x2, dt));
                path.line_to((x2, db));
                path.curve_to((x2 - dx, db), (x1 + dx, ib), (x1, ib));
                path.close_path();
                let color = Color::new([col[0], col[1], col[2], 0.42]);
                scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &path);
            }
        });
        // Listo el frame: vaciar para que el próximo registre de cero.
        REGISTRO.with(|reg| reg.borrow_mut().clear());
    })
}

/// Renderiza una lista de nodos hermanos en orden.
#[allow(clippy::too_many_arguments)]
fn render_nodos<Msg, FSel>(
    nodos: &[Nodo],
    atoms: &HashMap<Uuid, &NarrativeAtom>,
    palette: &Palette,
    cfg: &ConfigLienzos,
    seleccionado: Option<Uuid>,
    edicion: Option<&EdicionLienzo<Msg>>,
    ejecucion: Option<&EjecucionLienzo<Msg>>,
    on_select: &FSel,
) -> Vec<View<Msg>>
where
    Msg: Clone + 'static,
    FSel: Fn(Uuid) -> Msg + Clone + 'static,
{
    nodos
        .iter()
        .map(|n| match n {
            Nodo::Parrafo { atom } => render_parrafo(
                *atom,
                atoms,
                palette,
                cfg,
                seleccionado,
                edicion,
                ejecucion,
                on_select,
            ),
            Nodo::Seccion(s) => render_seccion(
                s,
                atoms,
                palette,
                cfg,
                seleccionado,
                edicion,
                ejecucion,
                on_select,
            ),
        })
        .collect()
}

/// `true` + el editor in-situ si `edicion` apunta a `atom`; `None` si no.
/// `font` es el tamaño de fuente con el que editar (nivel del átomo).
fn editor_si_corresponde<Msg>(
    atom: Uuid,
    font: f32,
    edicion: Option<&EdicionLienzo<Msg>>,
) -> Option<View<Msg>>
where
    Msg: Clone + 'static,
{
    let e = edicion?;
    if e.atom != atom {
        return None;
    }
    let onp = e.on_pointer.clone();
    let editor = text_editor_view::<Msg>(
        e.state,
        e.palette,
        EditorMetrics::for_font_size(font),
        VISIBLE_INLINE,
        move |ev| Some((onp)(ev)),
    );
    Some(
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: auto(),
            },
            min_size: Size {
                width: length(0.0_f32),
                height: length(font * 1.6),
            },
            ..Default::default()
        })
        .fill(e.palette.bg)
        .radius(3.0)
        .children(vec![editor]),
    )
}

/// Un párrafo de cuerpo: caja de texto clickeable a `font_base` — o el editor
/// in-situ si es el átomo en edición.
#[allow(clippy::too_many_arguments)]
fn render_parrafo<Msg, FSel>(
    atom: Uuid,
    atoms: &HashMap<Uuid, &NarrativeAtom>,
    palette: &Palette,
    cfg: &ConfigLienzos,
    seleccionado: Option<Uuid>,
    edicion: Option<&EdicionLienzo<Msg>>,
    ejecucion: Option<&EjecucionLienzo<Msg>>,
    on_select: &FSel,
) -> View<Msg>
where
    Msg: Clone + 'static,
    FSel: Fn(Uuid) -> Msg + Clone + 'static,
{
    if let Some(ed) = editor_si_corresponde(atom, cfg.font_base, edicion) {
        return ed;
    }
    let texto = atoms
        .get(&atom)
        .map(|a| a.content.to_string())
        .unwrap_or_default();
    // Celda LLM ejecutable (notebook embebido): caja con ▶ y salida inline.
    if celda_llm(&texto).is_some() {
        return render_celda(atom, &texto, palette, cfg, ejecucion, on_select);
    }
    let resaltado = seleccionado == Some(atom);

    let mut v = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .radius(3.0)
    .text_aligned(texto, cfg.font_base, palette.fg_text, Alignment::Start);

    if resaltado {
        v = v.fill(palette.bg_panel).border(1.0, palette.border_strong);
    }
    registrar_atom(v.on_click(on_select(atom)), atom)
}

/// Una celda LLM ejecutable: cabecera con ▶, el prompt (clickeable para editar)
/// y, si ya corrió, su salida inline. Es el "notebook" embebido en un lienzo.
fn render_celda<Msg, FSel>(
    atom: Uuid,
    texto: &str,
    palette: &Palette,
    cfg: &ConfigLienzos,
    ejecucion: Option<&EjecucionLienzo<Msg>>,
    on_select: &FSel,
) -> View<Msg>
where
    Msg: Clone + 'static,
    FSel: Fn(Uuid) -> Msg + Clone + 'static,
{
    let prompt = celda_llm(texto).unwrap_or_default();
    let acento = palette.border_strong;
    let [r, g, b, _] = acento.components;
    let fondo = Color::new([r, g, b, 0.06]);

    // Cabecera: rótulo + botón ▶ ejecutar (si hay contexto de ejecución).
    let mut cab: Vec<View<Msg>> = vec![View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: auto(),
            height: auto(),
        },
        ..Default::default()
    })
    .text_aligned("celda · llm", cfg.font_base * 0.8, palette.fg_muted, Alignment::Start)];
    if let Some(ej) = ejecucion {
        let onr = ej.on_run.clone();
        cab.push(
            View::new(Style {
                size: Size {
                    width: auto(),
                    height: auto(),
                },
                padding: Rect {
                    left: length(8.0_f32),
                    right: length(8.0_f32),
                    top: length(3.0_f32),
                    bottom: length(3.0_f32),
                },
                ..Default::default()
            })
            .fill(fondo)
            .radius(3.0)
            .border(1.0, acento)
            .text_aligned("▶ ejecutar", cfg.font_base * 0.85, acento, Alignment::Center)
            .on_click((onr)(atom)),
        );
    }
    let cabecera = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        align_items: Some(llimphi_ui::llimphi_layout::taffy::AlignItems::Center),
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(cab);

    // Cuerpo del prompt (clickeable para editar in-situ).
    let cuerpo_celda = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(prompt, cfg.font_base, palette.fg_text, Alignment::Start)
    .on_click(on_select(atom));

    let mut hijos = vec![cabecera, cuerpo_celda];

    // Salida (si ya se ejecutó).
    if let Some(salida) = ejecucion.and_then(|ej| ej.salidas.get(&atom)) {
        hijos.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: auto(),
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
            .radius(3.0)
            .text_aligned(
                format!("→ {salida}"),
                cfg.font_base,
                palette.fg_muted,
                Alignment::Start,
            ),
        );
    }

    let celda_box = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(5.0_f32),
        },
        padding: Rect {
            left: length(cfg.padding),
            right: length(cfg.padding * 0.5),
            top: length(cfg.padding * 0.6),
            bottom: length(cfg.padding * 0.6),
        },
        ..Default::default()
    })
    .fill(fondo)
    .border(1.0, acento)
    .radius(4.0)
    .children(hijos);
    registrar_atom(celda_box, atom)
}

/// Una sección: lienzo con cabecera (título a su tamaño de nivel) + contenido
/// anidado. El borde y una banda tienen el color de identidad del nivel.
#[allow(clippy::too_many_arguments)]
fn render_seccion<Msg, FSel>(
    s: &Seccion,
    atoms: &HashMap<Uuid, &NarrativeAtom>,
    palette: &Palette,
    cfg: &ConfigLienzos,
    seleccionado: Option<Uuid>,
    edicion: Option<&EdicionLienzo<Msg>>,
    ejecucion: Option<&EjecucionLienzo<Msg>>,
    on_select: &FSel,
) -> View<Msg>
where
    Msg: Clone + 'static,
    FSel: Fn(Uuid) -> Msg + Clone + 'static,
{
    let tinte = color_seccion(s.nivel as usize);
    let font_titulo = font_size_por_nivel(s.nivel, cfg.font_base);
    let resaltado = seleccionado == Some(s.titulo_atom);

    // Cabecera: el editor in-situ del título (a su fuente de nivel) si es el
    // átomo en edición; si no, el título estático, grande, clickeable.
    let cabecera = if let Some(ed) = editor_si_corresponde(s.titulo_atom, font_titulo, edicion) {
        ed
    } else {
        let titulo_color = if resaltado {
            palette.border_strong
        } else {
            palette.fg_text
        };
        let titulo_box = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: auto(),
            },
            padding: Rect {
                left: length(6.0_f32),
                right: length(6.0_f32),
                top: length(2.0_f32),
                bottom: length(4.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(s.titulo.clone(), font_titulo, titulo_color, Alignment::Start)
        .on_click(on_select(s.titulo_atom));
        registrar_atom(titulo_box, s.titulo_atom)
    };

    // Contenido anidado.
    let mut hijos: Vec<View<Msg>> = Vec::with_capacity(s.hijos.len() + 1);
    hijos.push(cabecera);
    hijos.extend(render_nodos(
        &s.hijos,
        atoms,
        palette,
        cfg,
        seleccionado,
        edicion,
        ejecucion,
        on_select,
    ));

    // Banda de color a la izquierda + borde tenue: el lienzo "contiene" a sus
    // hijos. Tinte translúcido del nivel como fondo muy sutil para separar
    // capas anidadas sin estridencia.
    let [r, g, b, _] = tinte.components;
    let fondo = Color::new([r, g, b, 0.05]);
    let borde = Color::new([r, g, b, 0.55]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(cfg.gap),
        },
        padding: Rect {
            left: length(cfg.padding),
            right: length(cfg.padding * 0.5),
            top: length(cfg.padding * 0.6),
            bottom: length(cfg.padding * 0.6),
        },
        ..Default::default()
    })
    .fill(fondo)
    .border(1.0, borde)
    .radius(4.0)
    .children(hijos)
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use pluma_cuerpo::Intencion;

    fn cuerpo_con(textos: &[&str]) -> (Cuerpo, Vec<NarrativeAtom>) {
        let mut c = Cuerpo::nuevo("es", "doc", Intencion::Original, 100);
        let atoms: Vec<NarrativeAtom> = textos
            .iter()
            .map(|t| NarrativeAtom::new(*t, "es"))
            .collect();
        for a in &atoms {
            c.agregar(a.id, 101);
        }
        (c, atoms)
    }

    fn mapa(atoms: &[NarrativeAtom]) -> HashMap<Uuid, &NarrativeAtom> {
        atoms.iter().map(|a| (a.id, a)).collect()
    }

    #[test]
    fn render_anidado_no_panica() {
        let (c, atoms) = cuerpo_con(&[
            "# Introducción",
            "Primer párrafo.",
            "## Motivación",
            "Hoy hay tres apps.",
        ]);
        let idx = mapa(&atoms);
        let _v: View<()> = lienzos_cuerpo_view(
            &c,
            &idx,
            &Palette::default(),
            &ConfigLienzos::default(),
            None,
            None,
            None,
            |_| (),
        );
    }

    #[test]
    fn celda_llm_detecta_fence() {
        assert_eq!(
            celda_llm("```llm\nescribe un haiku\n```"),
            Some("escribe un haiku".to_string())
        );
        assert_eq!(celda_llm("```LLM\nhola\n```"), Some("hola".to_string()));
        assert!(celda_llm("un párrafo normal").is_none());
        assert!(celda_llm("# título").is_none());
        assert!(celda_llm("```python\nx=1\n```").is_none());
    }

    #[test]
    fn multi_vacio_no_panica() {
        let idx: HashMap<Uuid, &NarrativeAtom> = HashMap::new();
        let _v: View<()> = lienzos_multi_view(
            &[],
            &idx,
            &Palette::default(),
            &ConfigLienzos::default(),
            0,
            None,
            None,
            None,
            &[],
            |_| (),
        );
    }

    #[test]
    fn multi_dos_cuerpos_no_panica() {
        let (a, atoms_a) = cuerpo_con(&["# A", "cuerpo a"]);
        let (b, atoms_b) = cuerpo_con(&["# B", "cuerpo b"]);
        let mut idx = mapa(&atoms_a);
        idx.extend(mapa(&atoms_b));
        let _v: View<()> = lienzos_multi_view(
            &[&a, &b],
            &idx,
            &Palette::default(),
            &ConfigLienzos::default(),
            0,
            None,
            None,
            None,
            &[],
            |_| (),
        );
    }
}
