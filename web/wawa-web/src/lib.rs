// =============================================================================
//  web/wawa-web — el host wasmi del kernel de wawa, vivo en el navegador.
// -----------------------------------------------------------------------------
//  Este crate compila a `wasm32-unknown-unknown` el SUBSISTEMA `wasm/` del
//  kernel: la matriz de capacidades, el shielding de combustible, la validacion
//  de fronteras de la memoria lineal del modulo huesped. La aplicacion del
//  userspace (hoy `hello_wasm`) se embebe en bytes y se ejecuta DENTRO de wasmi
//  como cualquier otra app de wawa — solo que el "kernel" que la hostea ya no
//  vive sobre x86 bare-metal sino sobre la WebAssembly del navegador.
//
//  Los handlers de capacidad reproducen al pie de la letra los de
//  03_ukupacha/wawa/wawa-kernel/src/wasm/env/presentacion.rs:
//
//    * `sys_render_frame(ptr, len)` — exige `len == ancho*alto*4`, valida el
//      rango contra la memoria lineal y copia el fotograma al lienzo del host.
//    * `sys_get_scancode() -> u32` — entrega el siguiente scancode de la cola
//      privada de la app, alimentada desde JS.
//
//  No esta el compositor todavia: el lienzo se entrega tal cual a JS, que lo
//  pinta en un <canvas>. La siguiente iteracion mete el compositor real.
// =============================================================================

use wasm_bindgen::prelude::*;
use wasmi::{Caller, Config, Engine, Error, Linker, Module, Store, TypedFunc};

// El binario WASM de la app — lo copia `build.sh` desde el target del crate
// hello_wasm. Embebiendolo aqui el binario web es autocontenido: una sola
// pieza que se descarga y arranca.
const HELLO_WASM: &[u8] = include_bytes!("../assets/hello_wasm.wasm");

// El lienzo natural que `hello_wasm` declara en su codigo (ANCHO/ALTO).
// El kernel real lo recibiria del manifiesto; aqui esta cableado.
const NAT_ANCHO: usize = 480;
const NAT_ALTO: usize = 560;

// El presupuesto de combustible de cada `tick`. Mismo orden de magnitud que
// el kernel (FUEL_ARRANQUE / FUEL_DINAMICO); con 20M cubre con holgura los
// 480*560*2 stores por fotograma de la rutina de pintado.
const FUEL_TICK: u64 = 20_000_000;

// El contexto que viaja con el Store de wasmi. Espeja `ContextoCapacidades`
// del kernel, pero recortado a lo que esta primera rebanada necesita.
struct ContextoApp {
    /// Cola de scancodes PS/2 Set 1, alimentada desde JS por
    /// `WawaWeb::enviar_scancode`. La app la lee con `sys_get_scancode`.
    canal: Vec<u32>,
    /// El fotograma mas reciente que la app entrego. Cada pixel es un u32 con
    /// la convencion `0x00_RR_GG_BB` (`grafico::Color::codificar`).
    lienzo: Vec<u32>,
}

#[wasm_bindgen]
pub struct WawaWeb {
    store: Store<ContextoApp>,
    tick_fn: TypedFunc<(), ()>,
}

#[wasm_bindgen]
impl WawaWeb {
    /// Forja el host: compila la app, enlaza las capacidades, instancia,
    /// concede combustible para `init` y lo invoca una vez.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<WawaWeb, JsValue> {
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);

        let modulo = Module::new(&engine, HELLO_WASM)
            .map_err(|e| JsValue::from_str(&format!("modulo invalido: {e}")))?;

        let mut store = Store::new(
            &engine,
            ContextoApp {
                canal: Vec::new(),
                lienzo: vec![0; NAT_ANCHO * NAT_ALTO],
            },
        );

        let mut enlazador = <Linker<ContextoApp>>::new(&engine);
        enlazar_capacidades(&mut enlazador).map_err(err_js)?;

        let instancia = enlazador
            .instantiate_and_start(&mut store, &modulo)
            .map_err(err_js)?;

        // `init`: el pintado del fondo de la app come bastante combustible,
        // se le concede el de arranque.
        store.set_fuel(FUEL_TICK).map_err(err_js)?;
        let init: TypedFunc<(), ()> = instancia
            .get_typed_func(&store, "init")
            .map_err(err_js)?;
        init.call(&mut store, ()).map_err(err_js)?;

        let tick_fn: TypedFunc<(), ()> = instancia
            .get_typed_func(&store, "tick")
            .map_err(err_js)?;

        Ok(WawaWeb { store, tick_fn })
    }

    /// Hace avanzar un fotograma de la app y devuelve el lienzo empaquetado en
    /// RGBA, listo para alimentar a `ImageData` del canvas. Si la app dispara
    /// una trampa (acceso fuera de limites, agota combustible...), se propaga
    /// como excepcion JS y el desalojo lo gestiona el llamador, como hace el
    /// kernel real.
    pub fn tick(&mut self) -> Result<Vec<u8>, JsValue> {
        self.store.set_fuel(FUEL_TICK).map_err(err_js)?;
        self.tick_fn
            .call(&mut self.store, ())
            .map_err(err_js)?;

        let lienzo = &self.store.data().lienzo;
        let mut salida = Vec::with_capacity(lienzo.len() * 4);
        for &p in lienzo {
            salida.push(((p >> 16) & 0xFF) as u8); // R
            salida.push(((p >> 8) & 0xFF) as u8);  // G
            salida.push((p & 0xFF) as u8);         // B
            salida.push(0xFF);                     // A
        }
        Ok(salida)
    }

    /// Empuja un scancode crudo al canal de la app. Cota a 64 para no acumular
    /// eventos viejos si el bucle de pintado se atrasa.
    pub fn enviar_scancode(&mut self, scancode: u32) {
        let canal = &mut self.store.data_mut().canal;
        if canal.len() < 64 {
            canal.push(scancode);
        }
    }

    #[wasm_bindgen(getter)]
    pub fn ancho(&self) -> u32 {
        NAT_ANCHO as u32
    }

    #[wasm_bindgen(getter)]
    pub fn alto(&self) -> u32 {
        NAT_ALTO as u32
    }
}

// -----------------------------------------------------------------------------
//  Capacidades — el equivalente de wawa-kernel/src/wasm/env/presentacion.rs.
// -----------------------------------------------------------------------------

fn enlazar_capacidades(enlazador: &mut Linker<ContextoApp>) -> Result<(), Error> {
    // CAPACIDAD :: sys_render_frame(ptr, len) — compositor de un fotograma.
    enlazador.func_wrap(
        "renaser",
        "sys_render_frame",
        |mut caller: Caller<'_, ContextoApp>, ptr: u32, len: u32| -> Result<(), Error> {
            let esperado = NAT_ANCHO * NAT_ALTO * 4;
            if len as usize != esperado {
                return Err(Error::new(
                    "WASM :: sys_render_frame con un fotograma ajeno al lienzo natural",
                ));
            }

            // Resolver la memoria lineal del modulo huesped.
            let memoria = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                Some(m) => m,
                None => return Err(Error::new("WASM :: el modulo no exporta memoria lineal")),
            };

            // `data_and_store_mut` entrega ambos borrows en un mismo paso: la
            // memoria del modulo (lectura) y los datos del Store (escritura)
            // del lienzo. Esquiva el conflicto que tendria que pedir uno
            // primero y el otro despues.
            let (datos, ctx) = memoria.data_and_store_mut(&mut caller);

            // Validacion infranqueable: el (ptr, len) debe caer dentro de la
            // memoria lineal del modulo huesped. Si no, se aborta la APP, no
            // el host — la trampa la propaga el llamador.
            let ptr_us = ptr as usize;
            let len_us = len as usize;
            let fin = ptr_us
                .checked_add(len_us)
                .ok_or_else(|| Error::new("WASM :: rango de sys_render_frame desborda usize"))?;
            if fin > datos.len() {
                return Err(Error::new(
                    "WASM :: sys_render_frame desbordo la memoria lineal del modulo",
                ));
            }

            // Decodificar los pixeles a u32 (LE) y volcarlos al lienzo del host.
            // En el kernel real el compositor cachea esto y lo recompone dentro
            // del marco asignado a la app; aqui, sin compositor, se entrega tal
            // cual a JS para pintar en el canvas.
            let fotograma = &datos[ptr_us..fin];
            for (i, chunk) in fotograma.chunks_exact(4).enumerate() {
                ctx.lienzo[i] =
                    u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            }
            Ok(())
        },
    )?;

    // CAPACIDAD :: sys_get_scancode() -> u32 — siguiente scancode del canal.
    enlazador.func_wrap(
        "renaser",
        "sys_get_scancode",
        |mut caller: Caller<'_, ContextoApp>| -> u32 {
            let canal = &mut caller.data_mut().canal;
            if canal.is_empty() {
                0
            } else {
                canal.remove(0)
            }
        },
    )?;

    Ok(())
}

// Pequeño puente para convertir errores de wasmi en JsValue conservando texto.
fn err_js<E: core::fmt::Display>(e: E) -> JsValue {
    JsValue::from_str(&format!("{e}"))
}
