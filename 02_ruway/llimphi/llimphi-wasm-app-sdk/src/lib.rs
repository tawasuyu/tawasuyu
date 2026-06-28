//! llimphi-wasm-app-sdk — lado guest de las apps WASM Tier 3.
//!
//! Una app implementa [`WasmApp`]: un `Model` propio, una transición
//! `update(&mut self, Msg)` y una vista pura `view(&self) -> WireNode`. La UI
//! se arma con el builder de [`llimphi_wire_view`] (`col`/`row`/`text` +
//! [`button`] de este crate, que serializa el `Msg` al `on_click`).
//!
//! [`export_wasm_app!`] genera el ABI que el host maneja:
//!
//! - `wasm_init()` — construye el `Model` y lo guarda en un global.
//! - `wasm_view() -> u64` — serializa el `WireNode`, lo deja en memoria lineal
//!   y devuelve `(ptr << 32) | len`.
//! - `wasm_alloc(len) -> u32` — reserva un buffer para que el host escriba el
//!   payload de un evento.
//! - `wasm_dispatch(ptr, len)` — decodifica el `Msg`, corre `update`.
//! - `wasm_free(ptr, len)` — libera el buffer de un `wasm_view` ya leído.
//!
//! El `Model` persiste entre frames porque el host mantiene viva la instancia
//! (un solo `Store`), igual que el modelo Elm normal de Llimphi.

// Re-exportamos el IR y su builder para que el guest sólo dependa de este SDK.
pub use llimphi_wire_view as wire;
pub use llimphi_wire_view::{
    col, leaf, row, spacer, text, Align, Dim, Dir, Justify, Rgba, TextAlign, WireNode, WireText,
};

/// Contrato de una app WASM Tier 3. El runtime lo maneja vía
/// [`export_wasm_app!`]; el autor sólo implementa estos tres métodos.
pub trait WasmApp {
    /// Mensajes de la app. Deben (de)serializar con postcard para cruzar la
    /// frontera: el `on_click` lleva los bytes, `dispatch` los decodifica.
    type Msg: serde::Serialize + serde::de::DeserializeOwned;

    /// Estado inicial.
    fn init() -> Self;

    /// Transición. Muta el `Model` en sitio.
    fn update(&mut self, msg: Self::Msg);

    /// Vista pura — el árbol serializable que el host materializa.
    fn view(&self) -> WireNode;
}

/// Botón: un nodo con `label` (texto + tamaño + color) y el `Msg` a emitir al
/// click ya serializado a postcard. Encadená estilo a gusto
/// (`.fill(..).radius(..).size(..)`), igual que el `.on_click(Msg)` de Llimphi.
///
/// ```ignore
/// button("+1", 28.0, [10, 30, 20, 255], &Msg::Increment)
///     .fill([60, 200, 130, 255])
///     .radius(12.0)
///     .size(Dim::Px(160.0), Dim::Px(56.0))
/// ```
pub fn button<M: serde::Serialize>(
    label: impl Into<String>,
    size: f32,
    color: Rgba,
    msg: &M,
) -> WireNode {
    let bytes = postcard::to_allocvec(msg).expect("postcard encode Msg");
    text(label, size, color).on_click_bytes(bytes)
}

/// Genera el ABI del guest a partir de un tipo que implementa [`WasmApp`].
///
/// ```ignore
/// struct Counter { n: u32 }
/// impl WasmApp for Counter { /* ... */ }
/// llimphi_wasm_app_sdk::export_wasm_app!(Counter);
/// ```
#[macro_export]
macro_rules! export_wasm_app {
    ($app:ty) => {
        // El Model vive acá entre frames. Single-thread (WASM), acceso unsafe
        // acotado a los puntos de entrada del host.
        static mut __WASM_APP: ::core::option::Option<$app> = ::core::option::Option::None;
        // Buffer de salida del último `wasm_view`, vivo hasta el `wasm_free`.
        static mut __WASM_OUT: ::std::vec::Vec<u8> = ::std::vec::Vec::new();

        #[no_mangle]
        pub extern "C" fn wasm_init() {
            let app = <$app as $crate::WasmApp>::init();
            unsafe {
                __WASM_APP = ::core::option::Option::Some(app);
            }
        }

        #[no_mangle]
        pub extern "C" fn wasm_view() -> u64 {
            let node = unsafe {
                let app = __WASM_APP
                    .as_ref()
                    .expect("wasm_view antes de wasm_init");
                $crate::WasmApp::view(app)
            };
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
            // El buffer es `__WASM_OUT`; se libera al sobrescribirse en el
            // próximo `wasm_view`. Lo soltamos ya para no retener memoria.
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
        pub extern "C" fn wasm_dispatch(ptr: u32, len: u32) {
            let bytes = unsafe {
                ::std::vec::Vec::from_raw_parts(ptr as *mut u8, len as usize, len as usize)
            };
            if let ::core::result::Result::Ok(msg) = $crate::postcard_from_slice::<
                <$app as $crate::WasmApp>::Msg,
            >(&bytes)
            {
                unsafe {
                    if let ::core::option::Option::Some(app) = __WASM_APP.as_mut() {
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
