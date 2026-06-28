//! llimphi-wire-view — IR serializable de un subset de `View<Msg>`.
//!
//! Un `WireNode` espeja la forma de `llimphi_compositor::View`: un árbol de
//! cajas con dirección flex, dimensiones, padding, relleno, radio, un texto
//! opcional y handlers. La diferencia clave: los callbacks **no cruzan la
//! frontera WASM**. En lugar de un `Fn() -> Msg`, cada nodo interactivo carga
//! `on_click: Option<Vec<u8>>` — los bytes postcard del `Msg` del guest. El
//! host rebota esos bytes a `dispatch` del guest, que los decodifica y corre
//! su `update`. Como en Llimphi `on_click` ya es `Option<Msg>` por valor (no
//! una clausura), el mapeo es directo.
//!
//! `no_std + alloc`: el mismo IR sirve para un guest no_std (incluso una app
//! wawa) y para el host con std.

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

/// Color RGBA de 8 bits por canal — el mismo formato que `Color::from_rgba8`.
pub type Rgba = [u8; 4];

/// Dimensión de un eje. Espeja `taffy::Dimension` en su subset útil.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Dim {
    /// Tamaño automático (contenido).
    Auto,
    /// Píxeles lógicos.
    Px(f32),
    /// Fracción del padre, `0.0..=1.0`.
    Pct(f32),
}

impl Default for Dim {
    fn default() -> Self {
        Dim::Auto
    }
}

/// Dirección del eje principal flex.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Dir {
    /// Caja simple (hoja o stack vertical). Se materializa como columna.
    Block,
    /// Fila — hijos en horizontal.
    Row,
    /// Columna — hijos en vertical.
    Column,
}

impl Default for Dir {
    fn default() -> Self {
        Dir::Block
    }
}

/// Alineación cruzada (`align-items`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Align {
    Start,
    Center,
    End,
    Stretch,
}

/// Distribución en el eje principal (`justify-content`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Justify {
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
}

/// Alineación horizontal del texto dentro de su caja.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
}

/// Texto a rasterizar dentro de un nodo.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct WireText {
    pub content: String,
    pub size: f32,
    pub color: Rgba,
    pub align: TextAlign,
    pub italic: bool,
}

/// Nodo del árbol. Recursivo, igual que `View<Msg>`. Construir con el builder
/// encadenable o con los constructores libres (`col`, `row`, `text`, `leaf`).
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct WireNode {
    pub dir: Dir,
    pub width: Dim,
    pub height: Dim,
    pub grow: f32,
    /// Gap entre hijos (ambos ejes), en px lógicos.
    pub gap: f32,
    /// Padding `[top, right, bottom, left]`, en px lógicos.
    pub padding: [f32; 4],
    pub align: Option<Align>,
    pub justify: Option<Justify>,
    pub fill: Option<Rgba>,
    pub radius: f32,
    pub text: Option<WireText>,
    /// Bytes postcard del `Msg` del guest a emitir al click. El host los rebota
    /// a `dispatch`; el guest los decodifica.
    pub on_click: Option<Vec<u8>>,
    pub children: Vec<WireNode>,
}

impl WireNode {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn dir(mut self, d: Dir) -> Self {
        self.dir = d;
        self
    }

    pub fn width(mut self, d: Dim) -> Self {
        self.width = d;
        self
    }

    pub fn height(mut self, d: Dim) -> Self {
        self.height = d;
        self
    }

    pub fn size(mut self, w: Dim, h: Dim) -> Self {
        self.width = w;
        self.height = h;
        self
    }

    pub fn grow(mut self, g: f32) -> Self {
        self.grow = g;
        self
    }

    pub fn gap(mut self, g: f32) -> Self {
        self.gap = g;
        self
    }

    /// Padding uniforme en los cuatro lados.
    pub fn pad(mut self, p: f32) -> Self {
        self.padding = [p; 4];
        self
    }

    /// Padding por lado `[top, right, bottom, left]`.
    pub fn padding(mut self, top: f32, right: f32, bottom: f32, left: f32) -> Self {
        self.padding = [top, right, bottom, left];
        self
    }

    pub fn align(mut self, a: Align) -> Self {
        self.align = Some(a);
        self
    }

    pub fn justify(mut self, j: Justify) -> Self {
        self.justify = Some(j);
        self
    }

    pub fn fill(mut self, color: Rgba) -> Self {
        self.fill = Some(color);
        self
    }

    pub fn radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }

    /// Fija el texto del nodo (alineación `Start`, sin itálica).
    pub fn label(mut self, content: impl Into<String>, size: f32, color: Rgba) -> Self {
        self.text = Some(WireText {
            content: content.into(),
            size,
            color,
            align: TextAlign::Start,
            italic: false,
        });
        self
    }

    /// Alinea el texto ya fijado (no-op si no hay texto).
    pub fn text_align(mut self, align: TextAlign) -> Self {
        if let Some(t) = self.text.as_mut() {
            t.align = align;
        }
        self
    }

    /// Adjunta los bytes postcard del `Msg` a emitir al click. El SDK del guest
    /// envuelve esto en un `button(label, .., msg)` que serializa por vos.
    pub fn on_click_bytes(mut self, bytes: Vec<u8>) -> Self {
        self.on_click = Some(bytes);
        self
    }

    pub fn children(mut self, children: Vec<WireNode>) -> Self {
        self.children = children;
        self
    }

    pub fn child(mut self, child: WireNode) -> Self {
        self.children.push(child);
        self
    }
}

/// Columna con hijos.
pub fn col(children: Vec<WireNode>) -> WireNode {
    WireNode::new().dir(Dir::Column).children(children)
}

/// Fila con hijos.
pub fn row(children: Vec<WireNode>) -> WireNode {
    WireNode::new().dir(Dir::Row).children(children)
}

/// Nodo de texto.
pub fn text(content: impl Into<String>, size: f32, color: Rgba) -> WireNode {
    WireNode::new().label(content, size, color)
}

/// Caja vacía (contenedor o separador). Usar `.grow(1.0)` para spacer flexible.
pub fn leaf() -> WireNode {
    WireNode::new()
}

/// Separador flexible (`flex-grow: 1`).
pub fn spacer() -> WireNode {
    WireNode::new().grow(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn round_trip_postcard() {
        let tree = col(vec![
            text("0", 160.0, [230, 240, 250, 255]).grow(1.0),
            row(vec![
                text("+1", 28.0, [10, 30, 20, 255])
                    .fill([60, 200, 130, 255])
                    .radius(12.0)
                    .on_click_bytes(vec![1, 2, 3]),
                text("reset", 22.0, [30, 10, 10, 255])
                    .fill([220, 80, 80, 255])
                    .radius(12.0)
                    .on_click_bytes(vec![9]),
            ])
            .gap(16.0)
            .justify(Justify::Center),
        ])
        .pad(32.0)
        .fill([20, 24, 32, 255]);

        let bytes = postcard::to_allocvec(&tree).unwrap();
        let back: WireNode = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(tree, back);
        // El click del primer botón carga sus bytes de Msg.
        assert_eq!(back.children[1].children[0].on_click.as_deref(), Some(&[1u8, 2, 3][..]));
    }
}
