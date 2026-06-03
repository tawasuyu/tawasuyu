//! `llimphi-widget-tree` — árbol con expand/collapse y selección.
//!
//! Análogo Llimphi al `nahual-widget-tree` GPUI. No mantiene estado
//! propio: el `Model` del App lleva el set de nodos expandidos + el
//! seleccionado, le pasa al widget la lista aplanada de filas (sólo
//! las visibles según el estado de expansión) y maneja los Msg de
//! toggle/select.
//!
//! Aplanar el árbol vive del lado del caller para no imponer una
//! representación específica (recursiva, plana con paths, etc.).
//!
//! Cada fila lleva su `depth` (para indentar), `has_children` (para
//! decidir si dibujar la flecha ▸/▾) y `expanded` (cuál de las dos).
//! Click en la flecha → `on_toggle`; click en el resto de la fila →
//! `on_select`; click derecho → `on_context` (si lo trae).
//!
//! Extras opcionales: un **icono gráfico** por fila (`icon`, cualquier
//! `View` — típicamente un mini-canvas vectorial) entre el chevron y el
//! label, y **líneas guía** de indentación (`TreeSpec::guides`).

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, Line as KurboLine, Stroke};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{PaintRect, View};

/// Paleta del árbol — un subset del `Theme` semántico, igual que los
/// otros widgets de Llimphi.
#[derive(Debug, Clone, Copy)]
pub struct TreePalette {
    pub bg_panel: Color,
    pub bg_selected: Color,
    pub bg_hover: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    pub fg_chevron: Color,
    /// Color de las líneas guía de indentación.
    pub guide: Color,
}

impl Default for TreePalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl TreePalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg_panel: t.bg_panel,
            bg_selected: t.bg_selected,
            bg_hover: t.bg_row_hover,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_chevron: t.fg_muted,
            guide: t.border,
        }
    }
}

/// Una fila del árbol — ya posicionada en la lista plana visible.
pub struct TreeRow<Msg> {
    pub label: String,
    /// Nivel de anidación (0 = raíz). Se traduce a indentación visual.
    pub depth: usize,
    /// Si el nodo tiene hijos. `false` = hoja; no se dibuja el chevron.
    pub has_children: bool,
    /// Estado actual del nodo. Ignorado si `has_children = false`.
    pub expanded: bool,
    /// Si esta fila es la seleccionada.
    pub selected: bool,
    /// Msg al hacer click en el chevron. Sólo se usa si `has_children`.
    pub on_toggle: Msg,
    /// Msg al hacer click en la fila (label o área alrededor).
    pub on_select: Msg,
    /// Icono gráfico opcional (cualquier `View`, p.ej. un mini-canvas
    /// vectorial) que se pinta entre el chevron y el label.
    pub icon: Option<View<Msg>>,
    /// Msg al hacer click derecho sobre la fila (menú contextual). `None`
    /// = sin menú contextual.
    pub on_context: Option<Msg>,
    /// Edición in-situ: si es `Some`, la fila se renderea con este
    /// `View` (típicamente un `text_input_view`) en el lugar del label,
    /// en vez del texto sólo-lectura. El chevron y la indentación se
    /// mantienen; el editor ocupa el slot elástico del label y no se le
    /// cablea `on_select` (las teclas las rutea el App). `None` = fila
    /// normal de sólo-lectura.
    pub editor: Option<View<Msg>>,
}

impl<Msg> TreeRow<Msg> {
    /// Constructor mínimo (sin icono / contexto / editor) — azúcar para
    /// callers que sólo quieren label + toggle + select.
    pub fn new(
        label: impl Into<String>,
        depth: usize,
        has_children: bool,
        expanded: bool,
        selected: bool,
        on_toggle: Msg,
        on_select: Msg,
    ) -> Self {
        Self {
            label: label.into(),
            depth,
            has_children,
            expanded,
            selected,
            on_toggle,
            on_select,
            icon: None,
            on_context: None,
            editor: None,
        }
    }

    pub fn with_icon(mut self, icon: View<Msg>) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn with_context(mut self, msg: Msg) -> Self {
        self.on_context = Some(msg);
        self
    }

    pub fn with_editor(mut self, editor: View<Msg>) -> Self {
        self.editor = Some(editor);
        self
    }
}

/// Especificación completa del árbol a renderear.
pub struct TreeSpec<Msg> {
    pub rows: Vec<TreeRow<Msg>>,
    pub row_height: f32,
    pub indent_px: f32,
    pub palette: TreePalette,
    /// Dibujar líneas guía verticales de indentación.
    pub guides: bool,
}

impl<Msg> TreeSpec<Msg> {
    /// Spec con valores por defecto sensatos (row 22, indent 14, sin
    /// guías) — sólo hay que pasar filas y paleta.
    pub fn new(rows: Vec<TreeRow<Msg>>, palette: TreePalette) -> Self {
        Self {
            rows,
            row_height: 22.0,
            indent_px: 14.0,
            palette,
            guides: false,
        }
    }
}

/// Compone el árbol como `View<Msg>`. El contenedor activa `clip` para
/// que filas que excedan el rect se recorten — usar dentro de un panel
/// del tamaño deseado.
pub fn tree_view<Msg: Clone + 'static>(spec: TreeSpec<Msg>) -> View<Msg> {
    let TreeSpec {
        rows,
        row_height,
        indent_px,
        palette,
        guides,
    } = spec;

    let children: Vec<View<Msg>> = rows
        .into_iter()
        .map(|row| tree_row_view(row, row_height, indent_px, guides, &palette))
        .collect();

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .clip(true)
    .children(children)
}

fn tree_row_view<Msg: Clone + 'static>(
    row: TreeRow<Msg>,
    height: f32,
    indent_px: f32,
    guides: bool,
    palette: &TreePalette,
) -> View<Msg> {
    let bg = if row.selected {
        palette.bg_selected
    } else {
        palette.bg_panel
    };
    let indent = (row.depth as f32) * indent_px;

    // Chevron a la izquierda — 16px de ancho, ▸ si colapsado, ▾ si
    // expandido. Si es hoja, espacio en blanco del mismo ancho para que
    // los labels alineen. ASCII puro (`v`/`>`) por compat de fuentes.
    let chevron_label = if row.has_children {
        if row.expanded {
            "v"
        } else {
            ">"
        }
    } else {
        " "
    };
    let chevron_msg = if row.has_children {
        Some(row.on_toggle)
    } else {
        None
    };
    let mut chevron = View::new(Style {
        size: Size {
            width: length(16.0_f32),
            height: length(height),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text_aligned(
        chevron_label.to_string(),
        12.0,
        palette.fg_chevron,
        Alignment::Center,
    );
    if let Some(msg) = chevron_msg {
        chevron = chevron.hover_fill(palette.bg_hover).on_click(msg);
    }

    let mut row_children: Vec<View<Msg>> = vec![chevron];

    // Icono gráfico opcional, entre chevron y label.
    if let Some(icon) = row.icon {
        row_children.push(
            View::new(Style {
                size: Size {
                    width: length(20.0_f32),
                    height: length(height),
                },
                flex_shrink: 0.0,
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .children(vec![icon]),
        );
    }

    // Slot elástico del label: editor in-situ si la fila lo trae, o el
    // texto sólo-lectura clickeable en su defecto. Alto `auto` para que el
    // `align_items: Center` de la fila lo centre verticalmente.
    let label = if let Some(editor) = row.editor {
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(height),
            },
            flex_grow: 1.0,
            padding: Rect {
                left: length(4.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![editor])
    } else {
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            padding: Rect {
                left: length(4.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(row.label, 12.0, palette.fg_text, Alignment::Start)
        .on_click(row.on_select)
    };
    row_children.push(label);

    let mut v = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(height),
        },
        padding: Rect {
            left: length(8.0_f32 + indent),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(palette.bg_hover)
    .children(row_children);

    // Líneas guía de indentación, pintadas por debajo de los hijos.
    if guides && row.depth > 0 {
        let guide = palette.guide;
        let depth = row.depth;
        v = v.paint_with(move |scene, _ts, rect: PaintRect| {
            let stroke = Stroke::new(1.0);
            for k in 0..depth {
                let x = (rect.x + 8.0 + k as f32 * indent_px + 7.0) as f64;
                let line = KurboLine::new((x, rect.y as f64), (x, (rect.y + rect.h) as f64));
                scene.stroke(&stroke, Affine::IDENTITY, guide, None, &line);
            }
        });
    }

    if let Some(ctx) = row.on_context {
        v = v.on_right_click(ctx);
    }

    v
}
