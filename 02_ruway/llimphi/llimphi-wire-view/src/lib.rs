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

/// Identificador de un handler de evento. El guest lo asigna por frame (en su
/// `view`); el host lo rebota en `dispatch(event_id, payload)` y el guest mira
/// su tabla para reconstruir el `Msg`. Reemplaza el viejo modelo de "bytes del
/// Msg en el nodo": ahora el nodo lleva sólo el id, así un evento puede acarrear
/// un payload (texto tecleado, estado de un toggle) que el host inyecta.
pub type EventId = u32;

/// Lo que un evento acarrea de vuelta al guest. El host lo serializa (postcard)
/// y lo pasa a `dispatch`; el guest lo entrega al handler del `event_id`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EventPayload {
    /// Click/tap — sin datos. Handler `Unit(Msg)`.
    Click,
    /// Texto nuevo de un campo editable. Handler `Text(Fn(String)->Msg)`.
    Text(String),
    /// Nuevo estado de un checkbox/switch. Handler `Toggle(Fn(bool)->Msg)`.
    Toggle(bool),
    /// Nuevo valor de un slider. Handler `Value(Fn(f32)->Msg)`.
    Value(f32),
    /// Índice de la opción elegida en un dropdown. Handler `Select(Fn(u32)->Msg)`.
    Select(u32),
}

/// Campo de texto editable. El `value` es la fuente de verdad del guest; el host
/// lo pinta y, al teclear, emite `on_input` con el texto nuevo (modelo Elm: el
/// host no guarda estado de edición, sólo notifica).
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct WireInput {
    pub value: String,
    pub placeholder: String,
    /// Oculta el contenido (●●●) — para contraseñas.
    pub password: bool,
    /// Acepta varias líneas: Enter inserta `\n` en vez de ignorarse. El guest
    /// sigue siendo la fuente de verdad del texto (modelo value-driven).
    pub multiline: bool,
}

/// Slider: barra con un valor en `[min, max]`. Al clickear/arrastrar, el host
/// emite `on_value` con el valor nuevo según la posición.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct WireSlider {
    pub value: f32,
    pub min: f32,
    pub max: f32,
}

/// Dropdown: lista de opciones con una seleccionada. El host pinta el header con
/// la opción actual; al elegir otra, emite `on_select` con su índice.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct WireSelect {
    pub options: Vec<String>,
    pub selected: u32,
}

/// Grupo de radio: lista siempre visible con una opción marcada. Al clickear
/// otra, emite `on_radio` con su índice (mismo payload que el dropdown,
/// `Select(idx)`). Difiere del dropdown sólo en el render: todas las opciones se
/// ven a la vez con un indicador ◉/○, sin estado de apertura.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct WireRadio {
    pub options: Vec<String>,
    pub selected: u32,
}

/// Nodo del árbol. Recursivo, igual que `View<Msg>`. Construir con el builder
/// encadenable o con los constructores libres (`col`, `row`, `text`, `leaf`).
/// Los nodos interactivos los arma el `Ui` del SDK, que asigna los `EventId`.
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
    /// Campo editable, si este nodo es un input.
    pub input: Option<WireInput>,
    /// Estado de un checkbox, si este nodo es uno.
    pub toggle: Option<bool>,
    /// Slider, si este nodo es uno.
    pub slider: Option<WireSlider>,
    /// Dropdown, si este nodo es uno.
    pub select: Option<WireSelect>,
    /// Grupo de radio, si este nodo es uno.
    pub radio: Option<WireRadio>,
    /// Handler de click (evento sin payload).
    pub on_click: Option<EventId>,
    /// Handler del texto tecleado en `input`.
    pub on_input: Option<EventId>,
    /// Handler del cambio de `toggle`.
    pub on_toggle: Option<EventId>,
    /// Handler del valor de `slider`.
    pub on_value: Option<EventId>,
    /// Handler de la opción elegida en `select`.
    pub on_select: Option<EventId>,
    /// Handler de la opción elegida en `radio`.
    pub on_radio: Option<EventId>,
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

    /// Asocia el `EventId` del handler de click (lo asigna el `Ui` del SDK).
    pub fn on_click(mut self, id: EventId) -> Self {
        self.on_click = Some(id);
        self
    }

    /// Marca el nodo como campo editable.
    pub fn with_input(mut self, input: WireInput) -> Self {
        self.input = Some(input);
        self
    }

    /// `EventId` del handler que recibe el texto tecleado.
    pub fn on_input(mut self, id: EventId) -> Self {
        self.on_input = Some(id);
        self
    }

    /// Marca el nodo como checkbox con el estado dado.
    pub fn with_toggle(mut self, checked: bool) -> Self {
        self.toggle = Some(checked);
        self
    }

    /// `EventId` del handler que recibe el nuevo estado del checkbox.
    pub fn on_toggle(mut self, id: EventId) -> Self {
        self.on_toggle = Some(id);
        self
    }

    /// Marca el nodo como slider.
    pub fn with_slider(mut self, slider: WireSlider) -> Self {
        self.slider = Some(slider);
        self
    }

    /// `EventId` del handler que recibe el valor nuevo del slider.
    pub fn on_value(mut self, id: EventId) -> Self {
        self.on_value = Some(id);
        self
    }

    /// Marca el nodo como dropdown.
    pub fn with_select(mut self, select: WireSelect) -> Self {
        self.select = Some(select);
        self
    }

    /// `EventId` del handler que recibe el índice elegido en el dropdown.
    pub fn on_select(mut self, id: EventId) -> Self {
        self.on_select = Some(id);
        self
    }

    /// Marca el nodo como grupo de radio.
    pub fn with_radio(mut self, radio: WireRadio) -> Self {
        self.radio = Some(radio);
        self
    }

    /// `EventId` del handler que recibe el índice elegido en el grupo de radio.
    pub fn on_radio(mut self, id: EventId) -> Self {
        self.on_radio = Some(id);
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
            WireNode::new()
                .with_input(WireInput {
                    value: "hola".into(),
                    placeholder: "nombre…".into(),
                    password: false,
                    multiline: false,
                })
                .on_input(7),
            row(vec![
                text("+1", 28.0, [10, 30, 20, 255])
                    .fill([60, 200, 130, 255])
                    .radius(12.0)
                    .on_click(1),
                text("reset", 22.0, [30, 10, 10, 255])
                    .fill([220, 80, 80, 255])
                    .radius(12.0)
                    .on_click(2),
            ])
            .gap(16.0)
            .justify(Justify::Center),
        ])
        .pad(32.0)
        .fill([20, 24, 32, 255]);

        let bytes = postcard::to_allocvec(&tree).unwrap();
        let back: WireNode = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(tree, back);
        assert_eq!(back.children[1].on_input, Some(7));
        assert_eq!(back.children[2].children[0].on_click, Some(1));

        // EventPayload también round-trippea.
        let p = EventPayload::Text("hola".into());
        assert_eq!(
            postcard::from_bytes::<EventPayload>(&postcard::to_allocvec(&p).unwrap()).unwrap(),
            p
        );
    }
}
