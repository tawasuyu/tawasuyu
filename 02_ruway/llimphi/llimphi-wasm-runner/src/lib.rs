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
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_wire_view::{Align, Dim, Dir, Justify, TextAlign, WireInput, WireNode};
use wasmi::{Caller, CompilationMode, Config, Engine, Linker, Memory, Module, Store, TypedFunc};

pub use format::Permisos;
pub use llimphi_wire_view::{EventId, EventPayload};

/// Mensaje del host. Un evento lleva el `EventId` del control y su `EventPayload`
/// (Click/Text/Toggle); el host los rebota a `wasm_dispatch` y el guest
/// reconstruye su `Msg`. `Focus` cambia el input enfocado (no cruza la frontera).
#[derive(Clone, Debug)]
pub enum RunnerMsg {
    /// Evento de un control: id del handler + payload.
    Event(EventId, EventPayload),
    /// El foco de teclado pasó a este input (o a ninguno).
    Focus(Option<EventId>),
    /// Abrir/cerrar el dropdown con este `on_select` id (estado host-side).
    ToggleSelect(EventId),
}

/// Una app WASM guest cargada y viva. Mantené una por app: el `Model` del guest
/// reside en su instancia y persiste entre `dispatch`.
pub struct WasmGuest {
    store: Store<()>,
    memory: Memory,
    f_view: TypedFunc<(), u64>,
    f_alloc: TypedFunc<u32, u32>,
    f_dispatch: TypedFunc<(u32, u32, u32), ()>,
    f_free: TypedFunc<(u32, u32), ()>,
    /// Última vista decodificada — el host la materializa cada frame sin volver
    /// a cruzar la frontera salvo tras un `dispatch`.
    view: WireNode,
    /// Input con el foco de teclado (su `on_input` EventId), si hay alguno.
    focused: Option<EventId>,
    /// Dropdown abierto (su `on_select` EventId), si hay alguno.
    open_select: Option<EventId>,
}

impl WasmGuest {
    /// Instancia el `.wasm` y corre `wasm_init` + el primer `wasm_view`.
    ///
    /// `permisos` es el bitfield efectivo (típicamente
    /// `format::permisos_efectivos(declarados, concedidos)` que calcula
    /// `llimphi-wasm-dist` tras verificar la concesión Ed25519). Gatea qué host
    /// imports se enlazan: si el bit falta, la función no se registra y un guest
    /// que la importe **trap-ea al instanciar** — frontera física, no tabla.
    pub fn load(wasm_bytes: &[u8], permisos: Permisos) -> Result<Self, String> {
        // Eager: los traps de compilación salen acá, no en pleno frame. Igual
        // criterio que llimphi-plugin-host y el kernel de wawa.
        let mut config = Config::default();
        config.compilation_mode(CompilationMode::Eager);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, wasm_bytes).map_err(|e| format!("compilar wasm: {e}"))?;
        let mut store = Store::new(&engine, ());
        // Una app Tier 3 pura (sólo UI) no importa nada; las capacidades (red…)
        // se enlazan acá sólo si el permiso correspondiente está concedido.
        let linker = build_linker(&engine, permisos)?;
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
            .get_typed_func::<(u32, u32, u32), ()>(&store, "wasm_dispatch")
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
            focused: None,
            open_select: None,
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

    /// Rebota un evento al guest: serializa el `payload`, lo escribe y corre
    /// `wasm_dispatch(event_id, ptr, len)`, luego refresca la vista.
    pub fn dispatch(&mut self, event_id: EventId, payload: EventPayload) -> Result<(), String> {
        let bytes = postcard::to_allocvec(&payload).map_err(|e| format!("encode payload: {e}"))?;
        let len = bytes.len() as u32;
        let ptr = self
            .f_alloc
            .call(&mut self.store, len)
            .map_err(|e| format!("wasm_alloc trap: {e}"))?;
        self.memory
            .write(&mut self.store, ptr as usize, &bytes)
            .map_err(|e| format!("escribir payload: {e}"))?;
        self.f_dispatch
            .call(&mut self.store, (event_id, ptr, len))
            .map_err(|e| format!("wasm_dispatch trap: {e}"))?;
        self.refresh()
    }

    /// Aplica un `RunnerMsg`. Conveniencia para el `update` del host.
    pub fn apply(&mut self, msg: &RunnerMsg) -> Result<(), String> {
        match msg {
            RunnerMsg::Event(id, payload) => {
                // Cualquier evento cierra un dropdown abierto (elegir/clicar = cerrar).
                self.open_select = None;
                self.dispatch(*id, payload.clone())
            }
            RunnerMsg::Focus(f) => {
                self.focused = *f;
                Ok(())
            }
            RunnerMsg::ToggleSelect(id) => {
                self.open_select = if self.open_select == Some(*id) {
                    None
                } else {
                    Some(*id)
                };
                Ok(())
            }
        }
    }

    /// El input enfocado actualmente (su `on_input` EventId).
    pub fn focused(&self) -> Option<EventId> {
        self.focused
    }

    /// Traduce un evento de teclado a un `RunnerMsg` para el input enfocado:
    /// computa "valor actual + tecla → texto nuevo" (Backspace borra el último
    /// carácter; un carácter se anexa) y emite `Text`. Devuelve `None` si no hay
    /// foco o la tecla no edita. Modelo value-driven: el guest es la fuente de
    /// verdad del texto; el host no guarda buffer (cursor siempre al final, MVP).
    pub fn key_to_msg(&self, event: &KeyEvent) -> Option<RunnerMsg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        let id = self.focused?;
        let inp = find_input(&self.view, id)?;
        let nuevo = edit_value(&inp.value, &event.key, event.text.as_deref(), inp.multiline)?;
        Some(RunnerMsg::Event(id, EventPayload::Text(nuevo)))
    }

    /// `RunnerMsg` para fijar el foco (lo usa el `on_focus` del host).
    pub fn focus_msg(id: Option<u64>) -> RunnerMsg {
        RunnerMsg::Focus(id.map(|x| x as EventId))
    }

    /// Materializa la vista cacheada en un `View<RunnerMsg>` Llimphi real.
    pub fn render(&self) -> View<RunnerMsg> {
        wire_to_view(&self.view, self.focused, self.open_select)
    }
}

/// Computa el texto nuevo de un campo dado el actual y una tecla: Backspace borra
/// el último carácter; un carácter se anexa; Enter inserta `\n` sólo si el campo
/// es `multiline` (en uno de una línea no edita); otras teclas no editan
/// (`None`). Pura — el corazón del modelo value-driven, testeable sin `KeyEvent`.
pub fn edit_value(actual: &str, key: &Key, text: Option<&str>, multiline: bool) -> Option<String> {
    match key {
        Key::Named(NamedKey::Backspace) => {
            let mut s = actual.to_string();
            s.pop();
            Some(s)
        }
        Key::Named(NamedKey::Enter) if multiline => Some(format!("{actual}\n")),
        Key::Character(_) => text.map(|t| format!("{actual}{t}")),
        _ => None,
    }
}

/// Busca el `WireInput` cuyo `on_input` es `id` en el árbol.
fn find_input(node: &WireNode, id: EventId) -> Option<&WireInput> {
    if node.on_input == Some(id) {
        if let Some(inp) = &node.input {
            return Some(inp);
        }
    }
    node.children.iter().find_map(|c| find_input(c, id))
}

// Colores por defecto de los controles (el estilo de la caja viene del nodo).
const INPUT_TEXT: [u8; 4] = [230, 235, 245, 255];
const INPUT_PLACEHOLDER: [u8; 4] = [120, 130, 145, 255];
const INPUT_BORDER: [u8; 4] = [80, 90, 110, 255];
const INPUT_BORDER_FOCUS: [u8; 4] = [90, 160, 230, 255];
const SLIDER_TRACK: [u8; 4] = [40, 46, 58, 255];
const SLIDER_FILL: [u8; 4] = [90, 160, 230, 255];
const SELECT_BG: [u8; 4] = [28, 34, 44, 255];
const SELECT_ITEM: [u8; 4] = [34, 40, 52, 255];
const SELECT_SEL: [u8; 4] = [50, 70, 100, 255];

/// Materializa un [`WireNode`] en un `View<RunnerMsg>` Llimphi real,
/// recursivamente. `focused` = input con el foco; `open_select` = dropdown
/// abierto (para pintar su lista).
pub fn wire_to_view(
    node: &WireNode,
    focused: Option<EventId>,
    open_select: Option<EventId>,
) -> View<RunnerMsg> {
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

    if let Some(inp) = &node.input {
        // Campo editable: caja focusable que muestra value/placeholder + caret.
        let id = node.on_input.unwrap_or(0);
        let is_focused = focused == Some(id);
        let (shown, col) = display_input(inp, is_focused);
        let border = if is_focused { INPUT_BORDER_FOCUS } else { INPUT_BORDER };
        view = view
            .text(shown, 20.0, color(col))
            .border(1.5, color(border))
            .focusable(id as u64)
            .on_click(RunnerMsg::Focus(Some(id)));
    } else if let Some(checked) = node.toggle {
        // Checkbox: glifo clickable que alterna su estado.
        let id = node.on_toggle.unwrap_or(0);
        let glyph = if checked { "\u{2611}" } else { "\u{2610}" }; // ☑ / ☐
        view = view
            .text(glyph, 24.0, color(INPUT_TEXT))
            .on_click(RunnerMsg::Event(id, EventPayload::Toggle(!checked)));
    } else if let Some(sl) = &node.slider {
        // Slider: track con una barra de relleno proporcional; click-en-x fija
        // el valor. id/min/max van Copy a la clausura.
        let id = node.on_value.unwrap_or(0);
        let (min, max) = (sl.min, sl.max);
        let frac = if max > min {
            ((sl.value - min) / (max - min)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        view = view
            .fill(color(SLIDER_TRACK))
            .radius(6.0)
            .on_click_at(move |lx, _ly, w, _h| {
                let f = if w > 0.0 { (lx / w).clamp(0.0, 1.0) } else { 0.0 };
                Some(RunnerMsg::Event(id, EventPayload::Value(min + f * (max - min))))
            });
        let bar = View::new(Style {
            size: Size {
                width: percent(frac),
                height: percent(1.0),
            },
            ..Default::default()
        })
        .fill(color(SLIDER_FILL))
        .radius(6.0);
        view = view.children(vec![bar]);
    } else if let Some(sel) = &node.select {
        // Dropdown: header con la opción actual + lista inline cuando está abierto.
        let id = node.on_select.unwrap_or(0);
        let is_open = open_select == Some(id);
        let current = sel
            .options
            .get(sel.selected as usize)
            .cloned()
            .unwrap_or_else(|| "\u{2014}".into()); // —
        let arrow = if is_open { "\u{25b4}" } else { "\u{25be}" }; // ▴ / ▾
        let header = item_box(36.0)
            .fill(color(SELECT_BG))
            .border(1.0, color(INPUT_BORDER))
            .text(format!("{current}   {arrow}"), 18.0, color(INPUT_TEXT))
            .on_click(RunnerMsg::ToggleSelect(id));
        let mut kids = vec![header];
        if is_open {
            for (i, opt) in sel.options.iter().enumerate() {
                let oid = i as u32;
                let bg = if oid == sel.selected { SELECT_SEL } else { SELECT_ITEM };
                kids.push(
                    item_box(32.0)
                        .fill(color(bg))
                        .text(opt.clone(), 18.0, color(INPUT_TEXT))
                        .on_click(RunnerMsg::Event(id, EventPayload::Select(oid))),
                );
            }
        }
        view = view.children(kids);
    } else if let Some(rad) = &node.radio {
        // Grupo de radio: todas las opciones visibles, la marcada con ◉. Cada
        // fila emite Select(idx) (mismo payload que el dropdown).
        let id = node.on_radio.unwrap_or(0);
        let kids: Vec<View<RunnerMsg>> = rad
            .options
            .iter()
            .enumerate()
            .map(|(i, opt)| {
                let oid = i as u32;
                let marcado = oid == rad.selected;
                let glyph = if marcado { "\u{25c9}" } else { "\u{25cb}" }; // ◉ / ○
                let col = if marcado { SLIDER_FILL } else { INPUT_TEXT };
                item_box(30.0)
                    .text(format!("{glyph}  {opt}"), 18.0, color(col))
                    .on_click(RunnerMsg::Event(id, EventPayload::Select(oid)))
            })
            .collect();
        view = view.children(kids);
    } else {
        if let Some(t) = &node.text {
            view = view.text_aligned(
                t.content.clone(),
                t.size,
                color(t.color),
                map_text_align(t.align),
            );
        }
        if let Some(id) = node.on_click {
            view = view.on_click(RunnerMsg::Event(id, EventPayload::Click));
        }
    }

    if !node.children.is_empty() {
        view = view.children(
            node.children
                .iter()
                .map(|c| wire_to_view(c, focused, open_select))
                .collect(),
        );
    }
    view
}

/// Una caja de fila a `height` px, ancho completo, contenido centrado a la
/// izquierda con padding — el ladrillo del header/items del dropdown.
fn item_box(height: f32) -> View<RunnerMsg> {
    View::new(Style {
        size: Size {
            width: percent(1.0),
            height: length(height),
        },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(10.0),
            right: length(10.0),
            top: length(0.0),
            bottom: length(0.0),
        },
        ..Default::default()
    })
    .radius(6.0)
}

/// Texto a mostrar en un input y su color: value (o placeholder), enmascarado si
/// es password, con un caret `│` cuando tiene el foco.
fn display_input(inp: &WireInput, focused: bool) -> (String, [u8; 4]) {
    let body = if inp.value.is_empty() {
        if focused {
            String::new()
        } else {
            return (inp.placeholder.clone(), INPUT_PLACEHOLDER);
        }
    } else if inp.password {
        "\u{25cf}".repeat(inp.value.chars().count()) // ●
    } else {
        inp.value.clone()
    };
    let shown = if focused { format!("{body}\u{2502}") } else { body }; // │
    (shown, INPUT_TEXT)
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

// =====================================================================
// Host imports — gateados por Permisos (frontera física, espejo host de
// wawa::wasm::env). Namespace `"tawa"`. Si el bit no está, el import no se
// enlaza y el guest que lo use trap-ea al instanciar.
// =====================================================================

/// Construye el linker para un guest con los `permisos` dados. Las funciones
/// inocuas (log) van siempre; las que tocan recursos se enlazan sólo con su bit.
pub fn build_linker(engine: &Engine, permisos: Permisos) -> Result<Linker<()>, String> {
    let mut linker = Linker::<()>::new(engine);

    // `host_log` — siempre disponible: traza, no toca recursos.
    linker
        .func_wrap("tawa", "host_log", |caller: Caller<'_, ()>, ptr: i32, len: i32| {
            if let Some(s) = read_utf8(&caller, ptr, len) {
                eprintln!("[wasm] {s}");
            }
        })
        .map_err(|e| format!("enlazar host_log: {e}"))?;

    // `host_net_request` — pide bytes a la red por hash (futuro: sobre
    // BrahmanNet). Gateado por PERMISO_RED: sin él, no se enlaza. Hoy es un
    // stub que devuelve -1 (no implementado), pero la FRONTERA ya es real:
    // un guest sin el permiso ni siquiera instancia si lo importa.
    if permisos & format::PERMISO_RED != 0 {
        linker
            .func_wrap(
                "tawa",
                "host_net_request",
                |_caller: Caller<'_, ()>, _ptr: i32, _len: i32| -> i32 { -1 },
            )
            .map_err(|e| format!("enlazar host_net_request: {e}"))?;
    }

    Ok(linker)
}

fn read_utf8(caller: &Caller<'_, ()>, ptr: i32, len: i32) -> Option<String> {
    let memory = caller.get_export("memory")?.into_memory()?;
    let ptr = ptr.max(0) as usize;
    let len = len.max(0) as usize;
    let data = memory.data(caller);
    let end = ptr.checked_add(len)?;
    if end > data.len() {
        return None;
    }
    String::from_utf8(data[ptr..end].to_vec()).ok()
}
