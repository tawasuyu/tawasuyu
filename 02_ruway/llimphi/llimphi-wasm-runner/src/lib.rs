//! llimphi-wasm-runner — lado host de las apps WASM Tier 3.
//!
//! [`WasmGuest`] carga un `.wasm` que sigue el ABI de `llimphi-wasm-app-sdk`,
//! lo instancia con wasmi y corre su bucle Elm:
//!
//! 1. `wasm_init` construye el `Model` del guest (vive en la instancia).
//! 2. `wasm_view` devuelve un [`WireNode`] serializado; lo deserializamos y lo
//!    cacheamos.
//! 3. [`wire_to_view`] lo materializa en un `View<RunnerMsg>` Llimphi real.
//! 4. Un click emite `RunnerMsg::Guest(bytes)`; [`WasmGuest::dispatch`] rebota
//!    esos bytes a `wasm_dispatch` y refresca la vista.
//!
//! El host mantiene un único `Store` vivo por app, así el `Model` persiste
//! entre frames — exactamente el contrato Elm de Llimphi, sólo que la
//! transición corre del otro lado de la frontera WASM.
//!
//! Es el gemelo, del lado Linux, del ejecutor de apps WASM de wawa
//! (`sys_render_frame` + caps por frontera física): el mismo espíritu de
//! "abrir apps servidas/distribuidas", pero pintando con Llimphi en vez de un
//! framebuffer crudo.

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Rect, Size, Style},
    AlignItems, JustifyContent,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;
use llimphi_wire_view::{Align, Dim, Dir, Justify, TextAlign, WireNode};
use wasmi::{CompilationMode, Config, Engine, Linker, Memory, Module, Store, TypedFunc};

/// Mensaje del host. La única variante lleva los bytes del `Msg` del guest, tal
/// como vinieron en el `on_click` del `WireNode`. El host no los interpreta —
/// los rebota a `wasm_dispatch`.
#[derive(Clone, Debug)]
pub enum RunnerMsg {
    /// Un click sobre un nodo: bytes postcard del `Msg` del guest.
    Guest(Vec<u8>),
}

/// Una app WASM guest cargada y viva. Mantené una por app: el `Model` del guest
/// reside en su instancia y persiste entre `dispatch`.
pub struct WasmGuest {
    store: Store<()>,
    memory: Memory,
    f_view: TypedFunc<(), u64>,
    f_alloc: TypedFunc<u32, u32>,
    f_dispatch: TypedFunc<(u32, u32), ()>,
    f_free: TypedFunc<(u32, u32), ()>,
    /// Última vista decodificada — el host la materializa cada frame sin volver
    /// a cruzar la frontera salvo tras un `dispatch`.
    view: WireNode,
}

impl WasmGuest {
    /// Instancia el `.wasm` y corre `wasm_init` + el primer `wasm_view`.
    pub fn load(wasm_bytes: &[u8]) -> Result<Self, String> {
        // Eager: los traps de compilación salen acá, no en pleno frame. Igual
        // criterio que llimphi-plugin-host y el kernel de wawa.
        let mut config = Config::default();
        config.compilation_mode(CompilationMode::Eager);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, wasm_bytes).map_err(|e| format!("compilar wasm: {e}"))?;
        let mut store = Store::new(&engine, ());
        // Linker vacío: una app Tier 3 pura (sólo UI) no importa nada del host.
        // Las capacidades (fs/red) se sumarían acá, gateadas por Permissions,
        // igual que el gateo por import de plugin-host.
        let linker = Linker::<()>::new(&engine);
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| format!("instanciar wasm: {e}"))?;

        let memory = instance
            .get_memory(&store, "memory")
            .ok_or("el guest no exporta `memory`")?;
        let f_init = instance
            .get_typed_func::<(), ()>(&store, "wasm_init")
            .map_err(|e| format!("export `wasm_init`: {e}"))?;
        let f_view = instance
            .get_typed_func::<(), u64>(&store, "wasm_view")
            .map_err(|e| format!("export `wasm_view`: {e}"))?;
        let f_alloc = instance
            .get_typed_func::<u32, u32>(&store, "wasm_alloc")
            .map_err(|e| format!("export `wasm_alloc`: {e}"))?;
        let f_dispatch = instance
            .get_typed_func::<(u32, u32), ()>(&store, "wasm_dispatch")
            .map_err(|e| format!("export `wasm_dispatch`: {e}"))?;
        let f_free = instance
            .get_typed_func::<(u32, u32), ()>(&store, "wasm_free")
            .map_err(|e| format!("export `wasm_free`: {e}"))?;

        f_init
            .call(&mut store, ())
            .map_err(|e| format!("wasm_init trap: {e}"))?;

        let mut guest = WasmGuest {
            store,
            memory,
            f_view,
            f_alloc,
            f_dispatch,
            f_free,
            view: WireNode::default(),
        };
        guest.refresh()?;
        Ok(guest)
    }

    /// La última vista decodificada del guest.
    pub fn view(&self) -> &WireNode {
        &self.view
    }

    /// Vuelve a pedir `wasm_view` y decodifica el `WireNode`.
    fn refresh(&mut self) -> Result<(), String> {
        let packed = self
            .f_view
            .call(&mut self.store, ())
            .map_err(|e| format!("wasm_view trap: {e}"))?;
        let ptr = (packed >> 32) as usize;
        let len = (packed & 0xffff_ffff) as usize;
        let data = self.memory.data(&self.store);
        let end = ptr.checked_add(len).ok_or("wasm_view: rango desbordado")?;
        if end > data.len() {
            return Err("wasm_view: rango fuera de la memoria del guest".into());
        }
        let bytes = data[ptr..end].to_vec();
        // Soltamos el buffer del guest antes de propagar errores de decodificación.
        let _ = self.f_free.call(&mut self.store, (ptr as u32, len as u32));
        self.view = postcard::from_bytes(&bytes).map_err(|e| format!("decodificar WireNode: {e}"))?;
        Ok(())
    }

    /// Rebota un evento al guest: escribe los bytes del `Msg`, corre
    /// `wasm_dispatch` y refresca la vista.
    pub fn dispatch(&mut self, msg_bytes: &[u8]) -> Result<(), String> {
        let len = msg_bytes.len() as u32;
        let ptr = self
            .f_alloc
            .call(&mut self.store, len)
            .map_err(|e| format!("wasm_alloc trap: {e}"))?;
        self.memory
            .write(&mut self.store, ptr as usize, msg_bytes)
            .map_err(|e| format!("escribir payload: {e}"))?;
        self.f_dispatch
            .call(&mut self.store, (ptr, len))
            .map_err(|e| format!("wasm_dispatch trap: {e}"))?;
        self.refresh()
    }

    /// Aplica un `RunnerMsg`. Conveniencia para el `update` del host.
    pub fn apply(&mut self, msg: &RunnerMsg) -> Result<(), String> {
        match msg {
            RunnerMsg::Guest(bytes) => self.dispatch(bytes),
        }
    }

    /// Materializa la vista cacheada en un `View<RunnerMsg>` Llimphi real.
    pub fn render(&self) -> View<RunnerMsg> {
        wire_to_view(&self.view)
    }
}

/// Materializa un [`WireNode`] en un `View<RunnerMsg>` Llimphi real,
/// recursivamente. Los `on_click` se convierten en `RunnerMsg::Guest(bytes)`.
pub fn wire_to_view(node: &WireNode) -> View<RunnerMsg> {
    let mut style = Style {
        flex_direction: match node.dir {
            Dir::Row => FlexDirection::Row,
            Dir::Column | Dir::Block => FlexDirection::Column,
        },
        size: Size {
            width: dim(node.width),
            height: dim(node.height),
        },
        flex_grow: node.grow,
        ..Default::default()
    };
    if node.gap != 0.0 {
        style.gap = Size {
            width: length(node.gap),
            height: length(node.gap),
        };
    }
    let [pt, pr, pb, pl] = node.padding;
    style.padding = Rect {
        top: length(pt),
        right: length(pr),
        bottom: length(pb),
        left: length(pl),
    };
    if let Some(a) = node.align {
        style.align_items = Some(map_align(a));
    }
    if let Some(j) = node.justify {
        style.justify_content = Some(map_justify(j));
    }

    let mut view = View::new(style);
    if let Some(fill) = node.fill {
        view = view.fill(color(fill));
    }
    if node.radius != 0.0 {
        view = view.radius(node.radius as f64);
    }
    if let Some(t) = &node.text {
        view = view.text_aligned(t.content.clone(), t.size, color(t.color), map_text_align(t.align));
    }
    if let Some(bytes) = &node.on_click {
        view = view.on_click(RunnerMsg::Guest(bytes.clone()));
    }
    if !node.children.is_empty() {
        view = view.children(node.children.iter().map(wire_to_view).collect());
    }
    view
}

fn dim(d: Dim) -> Dimension {
    match d {
        Dim::Auto => Dimension::auto(),
        Dim::Px(v) => length(v),
        Dim::Pct(v) => percent(v),
    }
}

fn color(c: [u8; 4]) -> Color {
    Color::from_rgba8(c[0], c[1], c[2], c[3])
}

fn map_align(a: Align) -> AlignItems {
    match a {
        Align::Start => AlignItems::FlexStart,
        Align::Center => AlignItems::Center,
        Align::End => AlignItems::FlexEnd,
        Align::Stretch => AlignItems::Stretch,
    }
}

fn map_justify(j: Justify) -> JustifyContent {
    match j {
        Justify::Start => JustifyContent::FlexStart,
        Justify::Center => JustifyContent::Center,
        Justify::End => JustifyContent::FlexEnd,
        Justify::SpaceBetween => JustifyContent::SpaceBetween,
        Justify::SpaceAround => JustifyContent::SpaceAround,
    }
}

fn map_text_align(a: TextAlign) -> llimphi_ui::llimphi_text::Alignment {
    match a {
        TextAlign::Start => llimphi_ui::llimphi_text::Alignment::Start,
        TextAlign::Center => llimphi_ui::llimphi_text::Alignment::Center,
        TextAlign::End => llimphi_ui::llimphi_text::Alignment::End,
    }
}
