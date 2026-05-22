// =============================================================================
//  renaser :: kernel/src/wasm — Fase 4/5 :: el escudo de aislamiento (WASM)
// -----------------------------------------------------------------------------
//  Aqui renaser sustituye las costosas fronteras de hardware (la MMU, los
//  anillos de la CPU) por limites MATEMATICOS sobre el bytecode. Una aplicacion
//  WebAssembly se ejecuta en su propia memoria lineal; sus unicas puertas al
//  exterior son las capacidades que el enlazador del host le concede. Lo que
//  no este importado no existe: no hay camino fisico para ejecutarlo.
//
//  FASE 5 :: el aislamiento deja de ser solo ESPACIAL (memoria) y pasa a ser
//  tambien TEMPORAL (tiempo de CPU). Cada `tick` se ejecuta con un presupuesto
//  estricto de COMBUSTIBLE (fuel): si una app lo agota —un bucle infinito, un
//  trabajo desmedido—, el runtime lanza una trampa, el kernel recupera el mando
//  y la desaloja. Ningun modulo, por discolo que sea, secuestra el procesador.
// =============================================================================

pub mod env;

use wasmi::{
    CompilationMode, Config, Engine, Linker, Module, Store, StoreLimitsBuilder, TrapCode, TypedFunc,
};

use crate::grafico::{Color, RegionPantalla};
use env::ContextoCapacidades;

/// Combustible concedido a `init`. Cubre con holgura el pintado inicial del
/// fondo de una region a pantalla casi completa — un gasto unico, de arranque.
const FUEL_ARRANQUE: u64 = 20_000_000;

/// Combustible concedido a cada `tick`. Sobra para un fotograma honesto (unos
/// cientos de miles de operaciones); una app en bucle infinito lo agota en
/// milisegundos y es desalojada. Este numero ES el techo temporal del userspace.
const FUEL_FOTOGRAMA: u64 = 2_000_000;

/// Por que el kernel da por terminada —desaloja— una aplicacion WASM.
#[derive(Clone, Copy)]
pub enum FallaApp {
    /// El modulo no se pudo cargar, validar, enlazar o instanciar.
    Carga,
    /// La aplicacion agoto su combustible dentro de un `tick`: bucle infinito
    /// o trabajo desmedido. El guardarrail TEMPORAL en accion.
    SinCombustible,
    /// La aplicacion intento crecer su memoria lineal mas alla de su cuota.
    /// El guardarrail ESPACIAL en accion.
    SinMemoria,
    /// La aplicacion ejecuto una trampa: acceso fuera de limites, instruccion
    /// `unreachable`, una capacidad violada... su propio codigo la abortó.
    Trampa,
}

impl FallaApp {
    /// El color de la baliza de desalojo segun la causa de la falla: amarillo
    /// palido si reviento su techo de memoria, purpura para cualquier otra.
    pub fn color_baliza(self) -> Color {
        match self {
            FallaApp::SinMemoria => Color::DESALOJO_MEMORIA,
            _ => Color::DESALOJO,
        }
    }
}

/// Una aplicacion WebAssembly viva: su estado PERSISTE entre fotogramas. A
/// diferencia de la Fase 4 —que instanciaba y cedia el control de un gesto—,
/// aqui la instancia se conserva y el kernel la hace avanzar `tick` a `tick`.
pub struct AplicacionWasm {
    /// El almacen: todo el estado de ESTA instancia — su memoria lineal, sus
    /// globales y el contexto de capacidades con su region de pantalla.
    almacen: Store<ContextoCapacidades>,
    /// El punto de entrada de fotograma, ya resuelto y con seguridad de tipos.
    /// `TypedFunc` es un asa autosuficiente dentro del `Store`: conservada esta,
    /// el handle de la `Instance` no aporta nada y no se retiene.
    func_tick: TypedFunc<(), ()>,
    /// La region de pantalla de la app — su ventana, y donde se tatua su baliza
    /// de desalojo si llega a fallar.
    region: RegionPantalla,
}

impl AplicacionWasm {
    /// Carga, valida, instancia y arranca una aplicacion WASM aislada, ligada a
    /// una region de pantalla. Si algo falla en el camino, se devuelve la falla
    /// en lugar de incendiar el kernel.
    ///
    /// El nuevo ABI del userspace exige dos exportaciones: `init` —invocada una
    /// sola vez, aqui— y `tick` —un fotograma de trabajo, invocada despues por
    /// el reactor en cada pulso del reloj.
    ///
    /// `techo_memoria` es la cuota de memoria lineal de ESTA app, en bytes —
    /// desde la Fase 7 la dicta su `EntradaApp` del manifiesto. `indice_app` es
    /// su posicion en el manifiesto: su identidad para las capacidades de
    /// estado persistido (Fase 7c).
    pub fn cargar(
        bytecode: &[u8],
        region: RegionPantalla,
        techo_memoria: usize,
        indice_app: usize,
    ) -> Result<AplicacionWasm, FallaApp> {
        // 1. El motor, con metricas de combustible y compilacion ANTICIPADA: la
        //    traduccion del modulo ocurre ahora, de modo que el `fuel` mida
        //    despues solo EJECUCION, jamas compilacion diferida.
        let mut config = Config::default();
        config.consume_fuel(true);
        config.compilation_mode(CompilationMode::Eager);
        let motor = Engine::new(&config);

        // 2. Validar y traducir el modulo — ya instrumentado con fuel.
        let modulo = Module::new(&motor, bytecode).map_err(|_| FallaApp::Carga)?;

        // 3. El almacen, con el contexto de capacidades de ESTA app: su region
        //    de pantalla, su canal de teclado y su techo de memoria. El canal
        //    se crea ahora pero se inscribe en la difusion de la IRQ1 al final,
        //    ya con la app cargada: una carga fallida no deja canales huerfanos.
        let canal = crate::async_system::teclado::crear_canal();
        let limites = StoreLimitsBuilder::new()
            .memory_size(techo_memoria)
            // Una expansion denegada se convierte en TRAMPA, no en un -1 que la
            // app pudiera ignorar: asi el kernel la captura y la desaloja.
            .trap_on_grow_failure(true)
            .build();
        let mut almacen = Store::new(
            &motor,
            ContextoCapacidades {
                region,
                canal,
                limites,
                indice_app,
            },
        );
        // Ligar el limitador de recursos: `wasmi` lo consultara en cada
        // `memory.grow`, tambien durante la instanciacion.
        almacen.limiter(|contexto| &mut contexto.limites);
        // Dotar de combustible ANTES de instanciar: la instanciacion no debe
        // quedarse a cero y abortar.
        almacen.set_fuel(FUEL_ARRANQUE).map_err(|_| FallaApp::Carga)?;

        // 4. El enlazador y la matriz de capacidades (ver `env`).
        let mut enlazador: Linker<ContextoCapacidades> = Linker::new(&motor);
        env::enlazar_capacidades(&mut enlazador).map_err(|_| FallaApp::Carga)?;

        // 5. Instanciar, resolviendo las importaciones contra las capacidades.
        let instancia = enlazador
            .instantiate_and_start(&mut almacen, &modulo)
            .map_err(|_| FallaApp::Carga)?;

        // 6. Resolver los dos puntos del ABI de fotograma: `init` y `tick`.
        let func_init = instancia
            .get_typed_func::<(), ()>(&almacen, "init")
            .map_err(|_| FallaApp::Carga)?;
        let func_tick = instancia
            .get_typed_func::<(), ()>(&almacen, "tick")
            .map_err(|_| FallaApp::Carga)?;

        // 7. Arranque unico: `init` prepara el estado inicial de la aplicacion.
        almacen.set_fuel(FUEL_ARRANQUE).map_err(|_| FallaApp::Carga)?;
        func_init
            .call(&mut almacen, ())
            .map_err(|_| FallaApp::Carga)?;

        // 8. Con la app ya cargada e instanciada, inscribir su canal de teclado
        //    en la difusion de la IRQ1: desde aqui recibe cada pulsacion.
        crate::async_system::teclado::registrar_canal(&almacen.data().canal);

        Ok(AplicacionWasm {
            almacen,
            func_tick,
            region,
        })
    }

    /// Hace avanzar la aplicacion un fotograma. Recarga su presupuesto de
    /// combustible y le cede el control con `tick`. Si la app lo agota o ejecuta
    /// una trampa, el kernel recupera el mando y la falla se devuelve para que
    /// la tarea proceda al desalojo. El kernel nunca pierde el control.
    pub fn tick(&mut self) -> Result<(), FallaApp> {
        // Recargar el deposito: cada fotograma parte con su techo intacto.
        self.almacen
            .set_fuel(FUEL_FOTOGRAMA)
            .map_err(|_| FallaApp::Trampa)?;

        match self.func_tick.call(&mut self.almacen, ()) {
            Ok(()) => Ok(()),
            // `as_trap_code` da un codigo publico y univoco para cada causa:
            // `OutOfFuel` pliega toda variante de agotamiento de combustible;
            // `GrowthOperationLimited` es la cuota de memoria denegada.
            Err(error) => match error.as_trap_code() {
                Some(TrapCode::OutOfFuel) => Err(FallaApp::SinCombustible),
                Some(TrapCode::GrowthOperationLimited) => Err(FallaApp::SinMemoria),
                _ => Err(FallaApp::Trampa),
            },
        }
    }

    /// La region de pantalla asignada a la aplicacion.
    pub fn region(&self) -> RegionPantalla {
        self.region
    }
}

/// Reconciliacion del ciclo de vida. Cuando una `AplicacionWasm` muere —porque
/// fue desalojada y su tarea concluyo—, su canal de teclado debe darse de baja
/// de la difusion de la IRQ1. Sin esto, el manejador de interrupciones seguiria
/// empujando scancodes a una cola muerta: una fuga lenta pero segura.
impl Drop for AplicacionWasm {
    fn drop(&mut self) {
        crate::async_system::teclado::cerrar_canal(&self.almacen.data().canal);
    }
}
