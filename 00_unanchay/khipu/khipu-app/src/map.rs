//! `map` — el lienzo de pensamientos: geometría de cámara, colocación
//! determinista de notas (anclaje), detección de regiones emergentes y el
//! pintado del mapa (nodos que respiran por masa, filamentos, topónimos).
//!
//! El resto de la app le pide vistas y consultas; los helpers compartidos
//! (`current_mass`, `now_secs`, `CLUSTER_THRESHOLD`) vienen de `estado` y
//! `modelo`; `button` viene de `panels`.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Position, Rect, Size, Style},
    AlignItems, Dimension, JustifyContent,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle as KurboCircle, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::{draw_block, Alignment, TextBlock};
use llimphi_ui::{DragPhase, View};
use llimphi_widget_text_input::{text_input_view, TextInputPalette};
use khipu_core::{EmergentRegion, NoteId};

use crate::panels::button;
use crate::estado::{current_mass, now_secs};
use crate::modelo::{Focus, Model, Msg, CLUSTER_THRESHOLD};

/// Un nodo del mapa, ya resuelto a coordenadas de mundo + su masa viva.
/// Datos planos para viajar dentro de la closure de pintura.
pub(crate) struct MapNode {
    id: NoteId,
    /// Coordenadas de mundo (el domicilio fijo de la nota).
    x: f32,
    y: f32,
    /// Masa "vivida" en el instante del render: enciende el brillo y el
    /// tamaño. Decae con el tiempo → el mapa respira sin que toques nada.
    mass: f32,
    /// `false` si cayó bajo el horizonte (sólo se ve con archivo activo).
    visible: bool,
    color: Color,
    label: String,
}

/// Mundo → pantalla local (relativa al rect del lienzo). El centro del
/// rect es el ancla del zoom; `pan` se suma en mundo, luego se escala.
pub(crate) fn world_to_local(wx: f32, wy: f32, w: f32, h: f32, pan: (f32, f32), zoom: f32) -> (f32, f32) {
    (w * 0.5 + (wx + pan.0) * zoom, h * 0.5 + (wy + pan.1) * zoom)
}

/// Inversa de [`world_to_local`]: pantalla local → mundo. Para resolver
/// qué nota cae bajo un click.
pub(crate) fn local_to_world(lx: f32, ly: f32, w: f32, h: f32, pan: (f32, f32), zoom: f32) -> (f32, f32) {
    let z = zoom.max(1e-3);
    ((lx - w * 0.5) / z - pan.0, (ly - h * 0.5) / z - pan.1)
}

/// La nota colocada más cercana a un click en coords locales, dentro de un
/// radio de tolerancia (~18 px de pantalla). `None` si el click cae en el
/// vacío — así arrastrar el fondo no cambia la selección.
pub(crate) fn pick_note(model: &Model, lx: f32, ly: f32, w: f32, h: f32) -> Option<NoteId> {
    let (wx, wy) = local_to_world(lx, ly, w, h, model.cam_pan, model.cam_zoom);
    let now = now_secs();
    let mut best: Option<(NoteId, f32)> = None;
    for id in &model.order {
        let Some(n) = model.store.get(*id) else { continue };
        let Some((px, py)) = n.pos else { continue };
        if !model.show_archive {
            let m = current_mass(&model.gravity, n, now);
            if !model.gravity.is_visible(m) {
                continue;
            }
        }
        let d2 = (px - wx).powi(2) + (py - wy).powi(2);
        if best.map(|(_, bd)| d2 < bd).unwrap_or(true) {
            best = Some((*id, d2));
        }
    }
    let tol = (18.0 / model.cam_zoom.max(1e-3)).powi(2);
    best.filter(|(_, d2)| *d2 <= tol).map(|(id, _)| id)
}

/// Separación mínima entre nodos al colocarlos (coordenadas de mundo).
pub(crate) const MAP_MIN_SEP: f32 = 30.0;
/// Ángulo áureo en radianes — reparte determinísticamente lo que no tiene
/// parentela semántica sin amontonarlo.
pub(crate) const GOLDEN_ANGLE: f32 = 2.399_963_2;

/// Le da a `id` un domicilio fijo en el mapa, **una sola vez**: cae en el
/// baricentro de sus parientes semánticos (ponderado por afinidad) y, si
/// quedó pegada a otra nota, se separa apenas. Determinista y dependiente
/// sólo de las notas ya asentadas, así el orden de inserción es estable y
/// el mapa nunca se reacomoda solo.
pub(crate) fn place_note(model: &mut Model, id: NoteId) {
    if model.store.get(id).map(|n| n.pos.is_some()).unwrap_or(true) {
        return; // ya tiene domicilio (o no existe): no se mueve.
    }
    // Vecinos ya colocados: su afinidad con la nota nueva y su posición.
    let mut kin: Vec<(f32, (f32, f32))> = Vec::new();
    for other in &model.order {
        if *other == id {
            continue;
        }
        let Some(pos) = model.store.get(*other).and_then(|n| n.pos) else { continue };
        let aff = model.field.affinity(id, *other).unwrap_or(0.0).max(0.0);
        kin.push((aff, pos));
    }

    let target = if kin.is_empty() {
        (0.0, 0.0) // primera nota del cuaderno: centro del mundo.
    } else {
        let wsum: f32 = kin.iter().map(|(w, _)| *w).sum();
        if wsum > 1e-3 {
            // Cae junto a su parentela: baricentro ponderado por afinidad.
            let (mut tx, mut ty) = (0.0_f32, 0.0_f32);
            for (w, (x, y)) in &kin {
                tx += w * x;
                ty += w * y;
            }
            (tx / wsum, ty / wsum)
        } else {
            // Ortogonal a todo: anillo determinista por id, lejos del núcleo.
            let ang = id as f32 * GOLDEN_ANGLE;
            let rad = 180.0 + 14.0 * (id as f32).sqrt();
            (rad * ang.cos(), rad * ang.sin())
        }
    };

    // Separación: empuja el target hasta despegarlo de cada vecino cercano.
    let mut p = target;
    for _ in 0..12 {
        let mut moved = false;
        for (_, q) in &kin {
            let dx = p.0 - q.0;
            let dy = p.1 - q.1;
            let d = (dx * dx + dy * dy).sqrt();
            if d < MAP_MIN_SEP {
                let (ux, uy) = if d > 1e-3 {
                    (dx / d, dy / d)
                } else {
                    let a = id as f32 * GOLDEN_ANGLE;
                    (a.cos(), a.sin())
                };
                let push = MAP_MIN_SEP - d;
                p.0 += ux * push;
                p.1 += uy * push;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }

    model.store.set_pos(id, p.0, p.1);
}

/// Envuelve `child` como cajón absoluto pegado al borde izquierdo, alto
/// completo con un margen. El mapa de fondo sigue recibiendo pan/zoom en
/// el resto de la ventana; sólo los clicks sobre el cajón los come él.
pub(crate) fn overlay_left(child: View<Msg>, width: f32) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(8.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
            right: auto(),
        },
        size: Size {
            width: length(width),
            height: auto(),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![child])
}

/// Columna interna del editor: barra de cierre (× ⇒ deselecciona) arriba +
/// el editor abajo, sobre `bg_panel`. La comparten el overlay lateral y la
/// tarjeta anclada del zoom semántico.
pub(crate) fn editor_shell(child: View<Msg>, theme: &Theme) -> View<Msg> {
    let close = button("× cerrar", theme.bg_button, theme.fg_muted, Msg::Deselect);
    let close_row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        flex_shrink: 0.0,
        justify_content: Some(JustifyContent::End),
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(vec![close]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![close_row, child])
}

/// Envuelve `child` como panel absoluto pegado al borde derecho, alto
/// completo, con barra de cierre. El editor del nodo abierto cuando se lo
/// edita de lejos (zoom bajo): un fallback práctico al anclaje in-situ.
pub(crate) fn overlay_right(child: View<Msg>, width: f32, theme: &Theme) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: auto(),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
            right: length(8.0_f32),
        },
        size: Size {
            width: length(width),
            height: auto(),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .children(vec![editor_shell(child, theme)])
}

/// Mundo → pantalla local usando el último tamaño de lienzo conocido + la
/// cámara. La versión de `view()`, donde el rect real aún no se sabe.
pub(crate) fn world_screen(model: &Model, wx: f32, wy: f32) -> (f32, f32) {
    let (w, h) = model.canvas_size;
    world_to_local(wx, wy, w, h, model.cam_pan, model.cam_zoom)
}

/// Posición de pantalla (local al lienzo) del nodo `id`. `None` si la nota
/// no tiene domicilio todavía.
pub(crate) fn node_screen_pos(model: &Model, id: NoteId) -> Option<(f32, f32)> {
    let (wx, wy) = model.store.get(id).and_then(|n| n.pos)?;
    Some(world_screen(model, wx, wy))
}

/// Regiones emergentes que el mapa ofrece bautizar: clústeres densos sin
/// topónimo cerca, ya con un nombre propuesto del contenido. La detección
/// y la propuesta de nombre viven en `khipu-core` (lógica agnóstica); acá
/// sólo armamos el sustrato espacial (notas visibles + colocadas) y los
/// topónimos ya existentes (incluido el bautizo en curso, que cuenta como
/// "ya ofrecido" para no duplicar el chip).
pub(crate) fn emergent_regions(model: &Model) -> Vec<EmergentRegion> {
    let now = now_secs();
    // Notas visibles y colocadas — el filtro de masa/horizonte (física
    // temporal) se aplica acá, antes de pasar al core puramente espacial.
    let placed: Vec<&khipu_core::Note> = model
        .store
        .iter()
        .filter(|n| {
            n.pos.is_some()
                && (model.show_archive || model.gravity.is_visible(current_mass(&model.gravity, n, now)))
        })
        .collect();
    let clusters = model.field.clusters(CLUSTER_THRESHOLD);
    // Topónimos que ya "poseen" una zona: los bautizados + el bautizo en curso.
    let mut named_spots: Vec<(f32, f32)> = model.regions.iter().map(|r| (r.x, r.y)).collect();
    if let Some(spot) = model.naming {
        named_spots.push(spot);
    }
    khipu_core::emergent_regions(
        &placed,
        &clusters,
        &named_spots,
        khipu_core::REGION_MIN_MEMBERS,
        khipu_core::REGION_MATCH_DIST,
    )
}

/// Chip clickeable que ofrece bautizar el clúster denso en `(wx, wy)` con
/// el nombre propuesto (`✛ {name}`). Al click abre el input de bautizo en
/// esa coordenada, ya precargado con la sugerencia.
pub(crate) fn name_region_chip(wx: f32, wy: f32, name: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_button)
    .radius(12.0)
    .hover_fill(theme.bg_button_hover)
    .text_aligned(format!("✛ {name}"), 11.0, theme.fg_muted, Alignment::Center)
    .on_click(Msg::BeginNaming(wx, wy, name.to_string()))
}

/// Mini-input del bautizo en curso: una tarjeta con el campo de texto
/// enfocado. Enter confirma, Esc cancela (en `on_key`).
pub(crate) fn naming_input(model: &Model, input_palette: &TextInputPalette) -> View<Msg> {
    let input = text_input_view(
        &model.region_input,
        "nombre de la zona…",
        model.focus == Focus::Region,
        input_palette,
        Msg::Focus(Focus::Region),
    );
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(model.theme.bg_panel)
    .radius(8.0)
    .children(vec![input])
}

/// Posiciona `child` como vista absoluta de tamaño `(w, h)` centrada en la
/// pantalla `(sx, sy)`, clampeada al lienzo. Para chips y mini-inputs que
/// viven en el mapa (sugerencia de bautizo, input de nombre).
pub(crate) fn pinned(child: View<Msg>, sx: f32, sy: f32, w: f32, h: f32, canvas: (f32, f32)) -> View<Msg> {
    let left = (sx - w * 0.5).clamp(4.0, (canvas.0 - w - 4.0).max(4.0));
    let top = (sy - h * 0.5).clamp(4.0, (canvas.1 - h - 4.0).max(4.0));
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(left),
            top: length(top),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(w),
            height: length(h),
        },
        ..Default::default()
    })
    .children(vec![child])
}

/// La tarjeta del nodo abierto, anclada a su coordenada `(nx, ny)` de
/// pantalla: el zoom semántico hecho carne — el editor vive EN el lugar del
/// pensamiento, no en un panel aparte. Se clampea para no salirse del
/// lienzo. Hija del canvas, así pan/zoom la arrastran con el nodo.
pub(crate) fn node_card(child: View<Msg>, nx: f32, ny: f32, canvas: (f32, f32), theme: &Theme) -> View<Msg> {
    let (cw_max, ch_max) = canvas;
    let cw = 380.0_f32.min((cw_max - 16.0).max(220.0));
    let ch = 440.0_f32.min((ch_max - 16.0).max(200.0));
    // Anclada bajo el nodo, centrada en X, clampeada a la ventana.
    let left = (nx - cw * 0.5).clamp(8.0, (cw_max - cw - 8.0).max(8.0));
    let top = (ny + 16.0).clamp(8.0, (ch_max - ch - 8.0).max(8.0));

    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(left),
            top: length(top),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(cw),
            height: length(ch),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .radius(8.0)
    .children(vec![editor_shell(child, theme)])
}

pub(crate) fn gravity_panel(model: &Model, injected: Vec<View<Msg>>) -> View<Msg> {
    let theme = model.theme;
    let now = now_secs();
    let clusters = model.field.clusters(CLUSTER_THRESHOLD);
    let selected = model.selected;
    let pan = model.cam_pan;
    let zoom = model.cam_zoom;

    // Nodos colocados (los que ya tienen domicilio), con su masa viva.
    let mut nodes: Vec<MapNode> = Vec::new();
    for id in &model.order {
        let Some(n) = model.store.get(*id) else { continue };
        let Some((x, y)) = n.pos else { continue };
        let mass = current_mass(&model.gravity, n, now);
        let visible = model.gravity.is_visible(mass);
        if !visible && !model.show_archive {
            continue;
        }
        nodes.push(MapNode {
            id: *id,
            x,
            y,
            mass,
            visible,
            color: cluster_color(*id, &clusters, theme),
            label: short_label(&n.title),
        });
    }

    // Topónimos: las regiones bautizadas, para pintarlas como rótulos de
    // continente detrás de los nodos.
    let regions: Vec<(String, f32, f32)> = model
        .regions
        .iter()
        .map(|r| (r.name.clone(), r.x, r.y))
        .collect();

    // Filamentos del nodo seleccionado: sus parientes más afines ya
    // colocados. Elegir un pensamiento enciende sus vecinos (activación
    // por difusión) — el motor de serendipia.
    let mut links: Vec<((f32, f32), (f32, f32), f32)> = Vec::new();
    if let Some(sel) = selected {
        if let Some(sp) = model.store.get(sel).and_then(|n| n.pos) {
            for (nid, aff) in model.field.nearest(sel, 6) {
                if aff < 0.20 {
                    continue;
                }
                if let Some(np) = model.store.get(nid).and_then(|n| n.pos) {
                    links.push((sp, np, aff));
                }
            }
        }
    }

    let canvas = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .paint_with(move |scene, ts, rect| {
        paint_map(scene, ts, rect, &nodes, &links, &regions, selected, pan, zoom, theme);
    })
    .draggable_at(|phase, dx, dy, _lx, _ly| match phase {
        DragPhase::Move => Some(Msg::MapPan(dx, dy)),
        DragPhase::End => None,
    })
    .on_scroll(|_dx, dy| Some(Msg::MapZoom(dy)))
    .on_click_at(|lx, ly, w, h| Some(Msg::MapClick(lx, ly, w, h)))
    // La tarjeta del nodo abierto (zoom semántico) viaja como hija del
    // canvas: se pinta encima de los nodos y la cámara la arrastra con el
    // pensamiento al que pertenece.
    .children(injected);

    View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![canvas])
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_map(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    ts: &mut llimphi_ui::llimphi_text::Typesetter,
    rect: llimphi_ui::PaintRect,
    nodes: &[MapNode],
    links: &[((f32, f32), (f32, f32), f32)],
    regions: &[(String, f32, f32)],
    selected: Option<NoteId>,
    pan: (f32, f32),
    zoom: f32,
    theme: Theme,
) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    // Pantalla absoluta = origen del rect + pantalla local.
    let to_screen = |wx: f32, wy: f32| -> (f64, f64) {
        let (lx, ly) = world_to_local(wx, wy, rect.w, rect.h, pan, zoom);
        ((rect.x + lx) as f64, (rect.y + ly) as f64)
    };

    // Topónimos al fondo: el nombre de cada región, grande y tenue, como
    // rótulo de continente; un halo suave insinúa su territorio.
    for (name, rx, ry) in regions {
        let (cx, cy) = to_screen(*rx, *ry);
        let blob = KurboCircle::new((cx, cy), (96.0 * zoom as f64).max(34.0));
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(theme.accent, 0.05),
            None,
            &blob,
        );
        let size = (15.0 * zoom).clamp(11.0, 28.0);
        // Centrado aproximado: `simple` alinea a la izquierda en (x, y).
        let est_w = name.chars().count() as f64 * size as f64 * 0.52;
        draw_block(
            scene,
            ts,
            &TextBlock::simple(
                name,
                size,
                with_alpha(theme.fg_text, 0.30),
                (cx - est_w * 0.5, cy - size as f64 * 0.6),
            ),
        );
    }

    // Filamentos primero (debajo de los nodos). Más opacos cuanto más afín.
    for (a, b, aff) in links {
        let (ax, ay) = to_screen(a.0, a.1);
        let (bx, by) = to_screen(b.0, b.1);
        let mut path = BezPath::new();
        path.move_to((ax, ay));
        path.line_to((bx, by));
        let alpha = (0.18 + aff * 0.55).clamp(0.0, 0.85);
        scene.stroke(
            &Stroke::new((0.8 + *aff as f64 * 1.6).max(0.6)),
            Affine::IDENTITY,
            with_alpha(theme.accent, alpha),
            None,
            &path,
        );
    }

    // Nodos: tamaño y brillo crecen con la masa viva (el mapa respira).
    for n in nodes {
        let (px, py) = to_screen(n.x, n.y);
        let m = n.mass.clamp(0.0, 2.0);
        // Radio base por masa, escalado apenas por zoom para no inflarse.
        let r = (3.0 + m * 4.5) * (0.6 + 0.4 * zoom.clamp(0.5, 1.5));
        // Brillo: las notas frescas arden; las que se enfrían se apagan
        // hacia el fondo. Bajo el horizonte (archivo) van casi transparentes.
        let glow = if n.visible {
            (0.35 + m * 0.45).clamp(0.0, 1.0)
        } else {
            0.18
        };
        let color = with_alpha(n.color, glow);
        // Halo tenue alrededor de las notas más encendidas.
        if n.visible && m > 0.6 {
            let halo = KurboCircle::new((px, py), (r + 5.0) as f64);
            scene.fill(Fill::NonZero, Affine::IDENTITY, with_alpha(n.color, 0.10), None, &halo);
        }
        let circle = KurboCircle::new((px, py), r as f64);
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &circle);

        if selected == Some(n.id) {
            let ring = KurboCircle::new((px, py), (r + 3.0) as f64);
            scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, theme.accent, None, &ring);
        }

        // Etiqueta: sólo si el zoom da espacio o es la seleccionada — para
        // no saturar el mapa lejano. El texto sale del Typesetter.
        if (zoom >= 0.9 || selected == Some(n.id)) && n.visible {
            let lbl_col = with_alpha(theme.fg_text, (glow + 0.25).clamp(0.0, 1.0));
            draw_block(
                scene,
                ts,
                &TextBlock::simple(&n.label, 10.0, lbl_col, (px + r as f64 + 4.0, py - 7.0)),
            );
        }
    }
}

pub(crate) fn cluster_color(id: NoteId, clusters: &[Vec<NoteId>], theme: Theme) -> Color {
    let idx = clusters.iter().position(|c| c.contains(&id)).unwrap_or(0);
    // Paleta tomada del theme + matices generados por golden-ratio
    // sobre el hue del accent. Determinista por índice.
    let palette: [Color; 6] = [
        theme.accent,
        with_alpha(rotate_hue(theme.accent, 0.16), 1.0),
        with_alpha(rotate_hue(theme.accent, 0.33), 1.0),
        with_alpha(rotate_hue(theme.accent, 0.50), 1.0),
        with_alpha(rotate_hue(theme.accent, 0.66), 1.0),
        with_alpha(rotate_hue(theme.accent, 0.83), 1.0),
    ];
    palette[idx % palette.len()]
}

pub(crate) fn with_alpha(c: Color, alpha: f32) -> Color {
    let [r, g, b, _] = c.components;
    Color::new([r, g, b, alpha])
}

pub(crate) fn rotate_hue(c: Color, dh: f32) -> Color {
    // RGB → HSV → rota H → RGB. Aproximación, alpha fijo.
    let [r, g, b, a] = c.components;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let v = max;
    let s = if max <= 0.0 { 0.0 } else { (max - min) / max };
    let h = if (max - min).abs() < 1e-6 {
        0.0
    } else if max == r {
        ((g - b) / (max - min)) % 6.0
    } else if max == g {
        (b - r) / (max - min) + 2.0
    } else {
        (r - g) / (max - min) + 4.0
    };
    let h2 = ((h / 6.0) + dh).rem_euclid(1.0) * 6.0;
    let c2 = v * s;
    let x = c2 * (1.0 - ((h2 % 2.0) - 1.0).abs());
    let (r2, g2, b2) = match h2 as i32 {
        0 => (c2, x, 0.0),
        1 => (x, c2, 0.0),
        2 => (0.0, c2, x),
        3 => (0.0, x, c2),
        4 => (x, 0.0, c2),
        _ => (c2, 0.0, x),
    };
    let m = v - c2;
    Color::new([r2 + m, g2 + m, b2 + m, a])
}

pub(crate) fn short_label(s: &str) -> String {
    let mut out: String = s.chars().take(24).collect();
    if s.chars().count() > 24 {
        out.push('…');
    }
    out
}
