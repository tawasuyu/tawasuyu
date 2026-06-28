//! llimphi-wasm-app-sdk — lado guest de las apps WASM Tier 3.
//!
//! Una app implementa [`WasmApp`]: un `Model` propio, una transición
//! `update(&mut self, Msg)` y una vista `view(&self, &mut Ui) -> WireNode`. La
//! UI se arma con los constructores libres de [`llimphi_wire_view`]
//! (`col`/`row`/`text`) para lo no-interactivo y con el [`Ui`] para lo
//! interactivo (`button`/`text_input`/`checkbox`).
//!
//! **Eventos con payload.** El `Ui` asigna un [`EventId`] a cada control y guarda
//! su handler. El nodo lleva sólo el id; el host, al disparar, manda
//! `dispatch(event_id, payload)`. El guest mira su tabla y reconstruye el `Msg`:
//! un botón rinde su `Msg` fijo, un input rinde `f(texto_tecleado)`, un checkbox
//! `f(nuevo_estado)`. Por eso **`Msg` ya no cruza la frontera** — sólo el id y el
//! [`EventPayload`] —, así que `Msg` se libera de (de)serializar: basta `Clone`.
//!
//! [`export_wasm_app!`] genera el ABI: `wasm_init`, `wasm_view() -> u64`
//! (ptr<<32|len del `WireNode` postcard), `wasm_dispatch(event_id, ptr, len)`,
//! `wasm_alloc`, `wasm_free`. El `Model` y la tabla de handlers viven en un
//! global y persisten entre frames porque el host mantiene viva la instancia.

pub use llimphi_wire_view as wire;
pub use llimphi_wire_view::{
    col, leaf, row, spacer, text, Align, Dim, Dir, EventId, EventPayload, Justify, Rgba, TextAlign,
    WireInput, WireNode, WireSelect, WireSlider, WireText,
};

/// Handler de un evento, del lado guest. El `Ui` los guarda; el dispatch los
/// resuelve a un `Msg`.
pub enum Handler<Msg> {
    /// Click/tap → un `Msg` fijo.
    Unit(Msg),
    /// Texto tecleado → `Msg` construido con el texto nuevo.
    Text(Box<dyn Fn(String) -> Msg>),
    /// Toggle → `Msg` construido con el nuevo estado.
    Toggle(Box<dyn Fn(bool) -> Msg>),
    /// Slider → `Msg` construido con el valor nuevo.
    Value(Box<dyn Fn(f32) -> Msg>),
    /// Dropdown → `Msg` construido con el índice elegido.
    Select(Box<dyn Fn(u32) -> Msg>),
}

/// Contexto de construcción de UI: asigna [`EventId`]s y acumula los handlers
/// del frame. Se reconstruye en cada `view` y se entrega por `&mut`.
pub struct Ui<Msg> {
    handlers: Vec<Handler<Msg>>,
}

impl<Msg> Default for Ui<Msg> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Msg> Ui<Msg> {
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }

    fn register(&mut self, h: Handler<Msg>) -> EventId {
        let id = self.handlers.len() as EventId;
        self.handlers.push(h);
        id
    }

    /// Botón: nodo con `label` y el `Msg` a emitir al click. Encadená estilo a
    /// gusto (`.fill(..).radius(..).size(..)`).
    pub fn button(&mut self, label: impl Into<String>, size: f32, color: Rgba, msg: Msg) -> WireNode {
        let id = self.register(Handler::Unit(msg));
        text(label, size, color).on_click(id)
    }

    /// Hace clickable cualquier nodo ya construido, emitiendo `msg`.
    pub fn clickable(&mut self, node: WireNode, msg: Msg) -> WireNode {
        let id = self.register(Handler::Unit(msg));
        node.on_click(id)
    }

    /// Campo de texto editable. `on_input(texto)` construye el `Msg` con el texto
    /// nuevo en cada cambio. El `value` es la fuente de verdad (Elm): pintalo
    /// desde tu `Model`.
    pub fn text_input(
        &mut self,
        value: impl Into<String>,
        placeholder: impl Into<String>,
        on_input: impl Fn(String) -> Msg + 'static,
    ) -> WireNode {
        let id = self.register(Handler::Text(Box::new(on_input)));
        WireNode::new()
            .with_input(WireInput {
                value: value.into(),
                placeholder: placeholder.into(),
                password: false,
            })
            .on_input(id)
    }

    /// Igual que [`Self::text_input`] pero oculta el contenido (contraseñas).
    pub fn password_input(
        &mut self,
        value: impl Into<String>,
        placeholder: impl Into<String>,
        on_input: impl Fn(String) -> Msg + 'static,
    ) -> WireNode {
        let id = self.register(Handler::Text(Box::new(on_input)));
        WireNode::new()
            .with_input(WireInput {
                value: value.into(),
                placeholder: placeholder.into(),
                password: true,
            })
            .on_input(id)
    }

    /// Checkbox. `on_toggle(nuevo_estado)` construye el `Msg`.
    pub fn checkbox(&mut self, checked: bool, on_toggle: impl Fn(bool) -> Msg + 'static) -> WireNode {
        let id = self.register(Handler::Toggle(Box::new(on_toggle)));
        WireNode::new().with_toggle(checked).on_toggle(id)
    }

    /// Slider con `value` en `[min, max]`. `on_value(v)` construye el `Msg` con
    /// el valor nuevo al clickear/arrastrar la barra.
    pub fn slider(
        &mut self,
        value: f32,
        min: f32,
        max: f32,
        on_value: impl Fn(f32) -> Msg + 'static,
    ) -> WireNode {
        let id = self.register(Handler::Value(Box::new(on_value)));
        WireNode::new()
            .with_slider(WireSlider { value, min, max })
            .on_value(id)
    }

    /// Dropdown con `options` y la `selected` actual. `on_select(idx)` construye
    /// el `Msg` con el índice elegido.
    pub fn select(
        &mut self,
        options: Vec<String>,
        selected: u32,
        on_select: impl Fn(u32) -> Msg + 'static,
    ) -> WireNode {
        let id = self.register(Handler::Select(Box::new(on_select)));
        WireNode::new()
            .with_select(WireSelect { options, selected })
            .on_select(id)
    }

    /// Resuelve un evento a un `Msg`. Lo usa el runtime generado por la macro.
    pub fn resolve(&self, id: EventId, payload: EventPayload) -> Option<Msg>
    where
        Msg: Clone,
    {
        match self.handlers.get(id as usize)? {
            Handler::Unit(m) => Some(m.clone()),
            Handler::Text(f) => match payload {
                EventPayload::Text(s) => Some(f(s)),
                _ => None,
            },
            Handler::Toggle(f) => match payload {
                EventPayload::Toggle(b) => Some(f(b)),
                _ => None,
            },
            Handler::Value(f) => match payload {
                EventPayload::Value(v) => Some(f(v)),
                _ => None,
            },
            Handler::Select(f) => match payload {
                EventPayload::Select(i) => Some(f(i)),
                _ => None,
            },
        }
    }
}

/// Contrato de una app WASM Tier 3. El runtime lo maneja vía
/// [`export_wasm_app!`].
pub trait WasmApp {
    /// Mensajes de la app. Sólo `Clone` — no cruzan la frontera (lo hace el
    /// `EventPayload`), así que no necesitan (de)serializar.
    type Msg: Clone;

    /// Estado inicial.
    fn init() -> Self;

    /// Transición. Muta el `Model` en sitio.
    fn update(&mut self, msg: Self::Msg);

    /// Vista. Construí lo interactivo con `ui` (asigna los EventIds).
    fn view(&self, ui: &mut Ui<Self::Msg>) -> WireNode;
}

/// Genera el ABI del guest a partir de un tipo que implementa [`WasmApp`].
#[macro_export]
macro_rules! export_wasm_app {
    ($app:ty) => {
        // Model + la tabla de handlers del último `view`. Persisten entre frames.
        static mut __WASM_RT: ::core::option::Option<(
            $app,
            $crate::Ui<<$app as $crate::WasmApp>::Msg>,
        )> = ::core::option::Option::None;
        static mut __WASM_OUT: ::std::vec::Vec<u8> = ::std::vec::Vec::new();

        #[no_mangle]
        pub extern "C" fn wasm_init() {
            let app = <$app as $crate::WasmApp>::init();
            unsafe {
                __WASM_RT = ::core::option::Option::Some((app, $crate::Ui::new()));
            }
        }

        #[no_mangle]
        pub extern "C" fn wasm_view() -> u64 {
            let rt = unsafe { __WASM_RT.as_mut().expect("wasm_view antes de wasm_init") };
            let mut ui = $crate::Ui::new();
            let node = $crate::WasmApp::view(&rt.0, &mut ui);
            rt.1 = ui; // la tabla de este frame, para el próximo dispatch
            let bytes = $crate::postcard_to_vec(&node);
            let ptr = bytes.as_ptr() as u64;
            let len = bytes.len() as u64;
            unsafe {
                __WASM_OUT = bytes;
            }
            (ptr << 32) | len
        }

        #[no_mangle]
        pub extern "C" fn wasm_free(_ptr: u32, _len: u32) {
            unsafe {
                __WASM_OUT = ::std::vec::Vec::new();
            }
        }

        #[no_mangle]
        pub extern "C" fn wasm_alloc(len: u32) -> u32 {
            let mut buf = ::std::vec![0u8; len as usize];
            let ptr = buf.as_mut_ptr() as u32;
            ::std::mem::forget(buf);
            ptr
        }

        #[no_mangle]
        pub extern "C" fn wasm_dispatch(event_id: u32, ptr: u32, len: u32) {
            let bytes = unsafe {
                ::std::vec::Vec::from_raw_parts(ptr as *mut u8, len as usize, len as usize)
            };
            let payload = match $crate::postcard_from_slice::<$crate::EventPayload>(&bytes) {
                ::core::result::Result::Ok(p) => p,
                ::core::result::Result::Err(_) => return,
            };
            unsafe {
                if let ::core::option::Option::Some((app, ui)) = __WASM_RT.as_mut() {
                    if let ::core::option::Option::Some(msg) = ui.resolve(event_id, payload) {
                        $crate::WasmApp::update(app, msg);
                    }
                }
            }
        }
    };
}

/// Helper para que la macro no exija a cada guest declarar `postcard`.
#[doc(hidden)]
pub fn postcard_to_vec<T: serde::Serialize>(value: &T) -> std::vec::Vec<u8> {
    postcard::to_allocvec(value).expect("postcard encode")
}

/// Helper de decodificación para la macro.
#[doc(hidden)]
pub fn postcard_from_slice<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, postcard::Error> {
    postcard::from_bytes(bytes)
}
