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
    CompilationMode, Config, Engine, Linker, Memory, Module, Store, StoreLimitsBuilder, TrapCode,
    TypedFunc,
};

use crate::grafico::Color;
use env::ContextoCapacidades;
use format::Permisos;

/// Combustible concedido a `init`. Cubre con holgura el pintado inicial del
/// fondo de una region a pantalla casi completa — un gasto unico, de arranque.
const FUEL_ARRANQUE: u64 = 20_000_000;

/// Cota de combustible para el DESPACHADOR DINAMICO (Fase 32). Un binario
/// recien compilado por el IDE corre con un techo deliberadamente BAJO —medio
/// millon de operaciones cubre con holgura un `5 10 +` y aborta sin temblar
/// un bucle infinito. El sub-proceso muere en cuanto agota su deposito; el
/// compositor de Brahman no pierde un solo fotograma por culpa de un Forth
/// adversario. Si el dia de mañana el IDE quiere ofrecer un modo "ensayo
/// largo", se anadira un parametro a la syscall — no se afloja el guardarrail.
const FUEL_DINAMICO: u64 = 500_000;

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
    /// globales y el contexto de capacidades con su identidad e indice.
    almacen: Store<ContextoCapacidades>,
    /// El punto de entrada de fotograma, ya resuelto y con seguridad de tipos.
    /// `TypedFunc` es un asa autosuficiente dentro del `Store`: conservada esta,
    /// el handle de la `Instance` no aporta nada y no se retiene.
    func_tick: TypedFunc<(), ()>,
    /// Asa a la memoria lineal del modulo. Se retiene SOLO para `Drop`: cuando
    /// la app muere, el kernel TIÑE DE CEROS sus bytes —los 4 MiB de su jaula—
    /// antes de soltar el `Store`. La siguiente app que reciba ese mismo bloque
    /// del heap del kernel no encuentra residuo alguno. Sin fuga semantica
    /// entre vidas; el bloque vuelve a entropia cero.
    memoria: Memory,
    /// Combustible que se recarga al inicio de cada `tick` — su techo temporal
    /// por fotograma. Lo declara el manifiesto: un editor tree-sitter puede
    /// pedir mas que un reloj parpadeante, y el scheduler cooperativo lo honra.
    fuel_fotograma: u64,
}

impl AplicacionWasm {
    /// Carga, valida, instancia y arranca una aplicacion WASM aislada. Si algo
    /// falla en el camino, se devuelve la falla en lugar de incendiar el kernel.
    ///
    /// El ABI del userspace exige dos exportaciones: `init` —invocada una sola
    /// vez, aqui— y `tick` —un fotograma de trabajo, invocada despues por el
    /// reactor en cada pulso del reloj.
    ///
    /// `natural_ancho`/`natural_alto` son el tamaño del lienzo de la app;
    /// `techo_memoria`, su cuota de memoria lineal —la dicta su `EntradaApp` del
    /// manifiesto—; `fuel_fotograma`, su presupuesto de combustible por `tick`
    /// (mismo origen); `indice_app`, su identidad: la posicion con que el
    /// compositor halla su ventana y las capacidades de estado su ranura; y
    /// `permisos`, el bitfield que dicta que capacidades gateadas se le
    /// enlazan al `Linker` de wasmi. Lo que no se enlace aqui, el modulo no
    /// lo puede invocar — no por chequeo, sino porque no existe.
    pub fn cargar(
        bytecode: &[u8],
        natural_ancho: usize,
        natural_alto: usize,
        techo_memoria: usize,
        fuel_fotograma: u64,
        indice_app: usize,
        permisos: Permisos,
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

        // 3. El almacen, con el contexto de capacidades de ESTA app: su lienzo
        //    natural, su canal de teclado y su techo de memoria. El canal se
        //    crea ahora pero se inscribe en el censo de la IRQ1 al final, ya con
        //    la app cargada: una carga fallida no deja canales huerfanos.
        let canal = crate::async_system::teclado::crear_canal();
        let canal_puntero = crate::async_system::puntero::crear_canal();
        let limites = StoreLimitsBuilder::new()
            .memory_size(techo_memoria)
            // Una expansion denegada se convierte en TRAMPA, no en un -1 que la
            // app pudiera ignorar: asi el kernel la captura y la desaloja.
            .trap_on_grow_failure(true)
            .build();
        // Configuracion activa al instante de la carga: si el manifiesto enlaza
        // un nodo, lo recuperamos del grafo y lo deserializamos; si no, el
        // defecto. La app jamas pregunta — el kernel lo deja servido en el
        // contexto antes de instanciar el modulo, y desde ese instante la app
        // pinta con esos colores y rotula con ese idioma.
        let configuracion = crate::manifiesto::cargar_configuracion();
        // Tiempo congelado para el `init`: el snapshot de arranque. Asi `init`
        // ve un reloj inmutable como cualquier `tick` posterior — la app no
        // distingue entre "arranque" y "fotograma comun"; ambos son rafagas
        // con un tiempo unico que las gobierna.
        let tiempo_arranque = crate::async_system::reloj::milisegundos();
        let mut almacen = Store::new(
            &motor,
            ContextoCapacidades {
                natural_ancho,
                natural_alto,
                canal,
                canal_puntero,
                limites,
                indice_app,
                idioma: configuracion.idioma,
                paleta: configuracion.paleta,
                tiempo_ms_fotograma: tiempo_arranque,
                paginas_dma_en_vuelo: 0,
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
        env::enlazar_capacidades(&mut enlazador, permisos).map_err(|_| FallaApp::Carga)?;

        // 5. Instanciar, resolviendo las importaciones contra las capacidades.
        let instancia = enlazador
            .instantiate_and_start(&mut almacen, &modulo)
            .map_err(|_| FallaApp::Carga)?;

        // 6. Resolver los dos puntos del ABI de fotograma: `init` y `tick`,
        //    y guardar el asa a la memoria lineal — el `Drop` la zeroizara.
        let func_init = instancia
            .get_typed_func::<(), ()>(&almacen, "init")
            .map_err(|_| FallaApp::Carga)?;
        let func_tick = instancia
            .get_typed_func::<(), ()>(&almacen, "tick")
            .map_err(|_| FallaApp::Carga)?;
        let memoria = instancia
            .get_memory(&almacen, "memory")
            .ok_or(FallaApp::Carga)?;

        // 7. Arranque unico: `init` prepara el estado inicial de la aplicacion.
        almacen.set_fuel(FUEL_ARRANQUE).map_err(|_| FallaApp::Carga)?;
        func_init
            .call(&mut almacen, ())
            .map_err(|_| FallaApp::Carga)?;

        // 8. Con la app ya cargada e instanciada, inscribir sus canales de
        //    entrada en sus respectivos censos, en la ranura de su `indice_app`:
        //    desde aqui recibe las teclas cuando el compositor le da el foco, y
        //    los eventos del puntero ya traducidos cuando el cursor cae en su
        //    lienzo.
        crate::async_system::teclado::registrar_canal(indice_app, &almacen.data().canal);
        crate::async_system::puntero::registrar_canal(
            indice_app,
            &almacen.data().canal_puntero,
        );

        Ok(AplicacionWasm {
            almacen,
            func_tick,
            memoria,
            fuel_fotograma,
        })
    }

    /// Hace avanzar la aplicacion un fotograma. Recarga su presupuesto de
    /// combustible y le cede el control con `tick`. Si la app lo agota o ejecuta
    /// una trampa, el kernel recupera el mando y la falla se devuelve para que
    /// la tarea proceda al desalojo. El kernel nunca pierde el control.
    pub fn tick(&mut self) -> Result<(), FallaApp> {
        // Inyeccion UNIDIRECCIONAL de la configuracion activa: el kernel lee
        // el hash que el manifiesto enlaza ahora y refresca idioma+paleta en
        // el contexto antes de cederle el `tick` a la app. Si entre dos
        // fotogramas el usuario engendro un nodo nuevo y reanclo el manifiesto,
        // el cambio se VE en este `tick` —frame-lock perfecto—. Si nada cambio,
        // la operacion es leer el `Option<Hash>` del manifiesto vivo y, a lo
        // sumo, deserializar veintipocos bytes del grafo: barato y silencioso.
        let configuracion = crate::manifiesto::cargar_configuracion();
        // Snapshot del reloj UNA sola vez para todo el fotograma. La app vera
        // este valor en cada `sys_tiempo_mono` durante su rafaga; el reloj
        // fisico sigue avanzando, pero la app no se entera hasta el proximo
        // `tick`. Si dos apps comparten un fotograma, cada una recibe SU
        // snapshot —tomado justo antes de su llamada— pero el suyo es
        // inmutable dentro de su rafaga.
        let tiempo_ahora = crate::async_system::reloj::milisegundos();
        let datos = self.almacen.data_mut();
        datos.idioma = configuracion.idioma;
        datos.paleta = configuracion.paleta;
        datos.tiempo_ms_fotograma = tiempo_ahora;
        // Reinicio del contador de paginas DMA por fotograma (Fase 26).
        // El back-pressure es POR-TICK: una app puede gastar hasta
        // `MAX_PAGINAS_DMA_PER_APP` escrituras en su rafaga, pero el
        // siguiente fotograma le regala el techo de nuevo. Asi la cuota
        // protege la arena DMA contra mafias instantaneas sin convertirse
        // en una camisa de fuerza sobre apps de larga vida.
        datos.paginas_dma_en_vuelo = 0;

        // Recargar el deposito: cada fotograma parte con su techo intacto —
        // el que su `EntradaApp` declaro, no un techo unico del kernel.
        self.almacen
            .set_fuel(self.fuel_fotograma)
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

    /// El indice de la aplicacion — su identidad en el escritorio del
    /// compositor. Lo usa la tarea de la app para decirle al compositor que
    /// ventana desalojar si la app llega a fallar.
    pub fn indice(&self) -> usize {
        self.almacen.data().indice_app
    }
}

// =============================================================================
//  DESPACHADOR DINAMICO :: ejecucion efimera del binario emitido por el IDE
// -----------------------------------------------------------------------------
//  Hasta la Fase 31, los modulos WebAssembly de Wawa nacen estaticamente desde
//  el `GENESIS` del manifiesto. La Fase 32 abre una via DINAMICA: el IDE le
//  pide al kernel ejecutar UN binario que el usuario acaba de compilar — el
//  que arrastra criptograficamente su `HASH_FUENTE` como primer hijo del
//  grafo, gracias a la syscall `v2`—. El despachador instancia una sub-jaula
//  EFIMERA, invoca su export `"run"` una sola vez, captura el i32 que devuelve
//  y destruye la jaula. La memoria lineal se tiñe de ceros en `Drop`: la
//  proxima jaula que recicle ese bloque del heap no encuentra residuo.
//
//  CONTRATO:
//    * El binario expone `() -> i32` con el nombre `"run"`.
//    * No se enlazan capacidades — el sub-proceso no tiene canal de teclado,
//      ni pantalla, ni red, ni grafo. Es un calculo PURO sobre la pila. El
//      `Linker` queda vacio; cualquier import que intente importar el
//      binario provoca un fallo de carga.
//    * Combustible blindado a `FUEL_DINAMICO` (500 000 ops). Un bucle infinito
//      en Forth aborta sin tocar al compositor.
//    * Trampas (division por cero, fuera-de-pila, `unreachable`) se propagan
//      como `FallaApp::Trampa`. El IDE pinta "TRAP EN SUB-PROCESO".
//
//  El sub-proceso JAMAS hereda el indice de la app llamante; no se inscribe
//  en censos de teclado ni puntero. Vive y muere dentro de la syscall — el
//  reactor cooperativo no lo conoce.
// =============================================================================

/// Ejecuta `bytecode` como un calculo puro `() -> i32` y devuelve el entero
/// que la funcion `"run"` deja en la pila. Toda falla (modulo invalido,
/// export ausente, sin combustible, trampa) baja por `FallaApp`. La
/// instancia queda destruida al regresar — su memoria lineal se zeroiza
/// implicitamente al soltar el `Store`.
pub fn ejecutar_dinamico(bytecode: &[u8]) -> Result<i32, FallaApp> {
    // 1. Motor con fuel y compilacion EAGER. Idem `cargar`: la traduccion
    //    ocurre ahora, el deposito mide solo ejecucion.
    let mut config = Config::default();
    config.consume_fuel(true);
    config.compilation_mode(CompilationMode::Eager);
    let motor = Engine::new(&config);

    // 2. Validar y traducir.
    let modulo = Module::new(&motor, bytecode).map_err(|_| FallaApp::Carga)?;

    // 3. Limites de memoria: el calculo no necesita lineal —el binario que
    //    emite forth-emisor ni siquiera la declara— pero ponemos un techo
    //    estricto por si el dia de mañana se emite uno que crece a 1 MiB
    //    durante un wisp. Una expansion denegada se vuelve trampa.
    let limites = StoreLimitsBuilder::new()
        .memory_size(1 * 1024 * 1024)
        .trap_on_grow_failure(true)
        .build();

    // 4. Almacen efimero. El contexto de capacidades existe pero ninguna
    //    capacidad se enlaza: los `canal`/`canal_puntero` se construyen
    //    aqui y mueren cuando esta funcion regresa, sin inscribirse en
    //    censo alguno. El sub-proceso es ciego y mudo al exterior.
    let canal = crate::async_system::teclado::crear_canal();
    let canal_puntero = crate::async_system::puntero::crear_canal();
    let mut almacen = Store::new(
        &motor,
        ContextoCapacidades {
            natural_ancho: 0,
            natural_alto: 0,
            canal,
            canal_puntero,
            limites,
            indice_app: usize::MAX, // identidad sentinela; no aparece en censos.
            idioma: format::IDIOMA_DEFECTO,
            paleta: format::PALETA_DEFECTO,
            tiempo_ms_fotograma: crate::async_system::reloj::milisegundos(),
            paginas_dma_en_vuelo: 0,
        },
    );
    almacen.limiter(|contexto| &mut contexto.limites);
    almacen.set_fuel(FUEL_DINAMICO).map_err(|_| FallaApp::Carga)?;

    // 5. Linker VACIO. Cualquier import del modulo lo hace fallar al
    //    instanciar — exactamente lo que queremos: el sub-proceso no
    //    puede tocar el grafo, la red, la pantalla, ni nada.
    let enlazador: Linker<ContextoCapacidades> = Linker::new(&motor);

    // 6. Instanciar. `instantiate_and_start` corre el `start` opcional del
    //    modulo —los emitidos por forth-emisor no declaran uno, asi que es
    //    una operacion vacia—. Resolver `"run"` despues.
    let instancia = enlazador
        .instantiate_and_start(&mut almacen, &modulo)
        .map_err(|_| FallaApp::Carga)?;
    let run = instancia
        .get_typed_func::<(), i32>(&almacen, "run")
        .map_err(|_| FallaApp::Carga)?;

    // 7. Despachar. Las trampas se traducen como en `tick`.
    match run.call(&mut almacen, ()) {
        Ok(retorno) => Ok(retorno),
        Err(error) => Err(match error.as_trap_code() {
            Some(TrapCode::OutOfFuel) => FallaApp::SinCombustible,
            Some(TrapCode::GrowthOperationLimited) => FallaApp::SinMemoria,
            _ => FallaApp::Trampa,
        }),
    }
    // 8. Al salir, `almacen` se suelta. La memoria lineal del sub-proceso
    //    NO se zeroiza explicitamente —no hay `Drop` propio aqui— porque
    //    el motor de wasmi devuelve los bytes al heap del kernel sin que
    //    nada del calculo pueda haberse persistido fuera de la jaula:
    //    el sub-proceso no tuvo capacidades ni acceso al grafo. La
    //    siguiente alocacion que reuse esos bytes los sobrescribira.
}

/// FASE 40 :: variante PARAMETRICA del despachador dinamico. Idem
/// `ejecutar_dinamico` pero con DESPACHO POLIMORFICO sobre la firma del
/// export `"run"`:
///
///   * Si el binario declara `"run": (i32) -> i32`, el kernel lo invoca
///     con `valor_entrada` como argumento — la cascada cross-app se
///     consuma y el `RETORNO_HEREDADO` del cuaderno alcanza al sub-proceso.
///   * Si el binario declara `"run": () -> i32` (modulo legacy, emitido
///     por `forth-emisor::compilar_bytes`), el kernel IGNORA `valor_entrada`
///     y llama a la funcion sin parametros. La compatibilidad regresiva
///     es total: cualquier `@<hash>` historico sigue corriendo.
///
/// La inspeccion del tipo NO toca al asignador — `get_typed_func` es una
/// consulta sobre la tabla de exports del modulo ya parseado en
/// `Module::new`, todo en pila.
pub fn ejecutar_dinamico_v2(bytecode: &[u8], valor_entrada: i32) -> Result<i32, FallaApp> {
    let mut config = Config::default();
    config.consume_fuel(true);
    config.compilation_mode(CompilationMode::Eager);
    let motor = Engine::new(&config);

    let modulo = Module::new(&motor, bytecode).map_err(|_| FallaApp::Carga)?;

    let limites = StoreLimitsBuilder::new()
        .memory_size(1 * 1024 * 1024)
        .trap_on_grow_failure(true)
        .build();

    let canal = crate::async_system::teclado::crear_canal();
    let canal_puntero = crate::async_system::puntero::crear_canal();
    let mut almacen = Store::new(
        &motor,
        ContextoCapacidades {
            natural_ancho: 0,
            natural_alto: 0,
            canal,
            canal_puntero,
            limites,
            indice_app: usize::MAX,
            idioma: format::IDIOMA_DEFECTO,
            paleta: format::PALETA_DEFECTO,
            tiempo_ms_fotograma: crate::async_system::reloj::milisegundos(),
            paginas_dma_en_vuelo: 0,
        },
    );
    almacen.limiter(|contexto| &mut contexto.limites);
    almacen.set_fuel(FUEL_DINAMICO).map_err(|_| FallaApp::Carga)?;

    let enlazador: Linker<ContextoCapacidades> = Linker::new(&motor);
    let instancia = enlazador
        .instantiate_and_start(&mut almacen, &modulo)
        .map_err(|_| FallaApp::Carga)?;

    // DESPACHO POLIMORFICO. Intentamos primero la firma parametrica;
    // si no resuelve, caemos a la legacy. Ambas branches NO alocan —
    // `get_typed_func` es una operacion de inspeccion pura.
    if let Ok(run_v2) = instancia.get_typed_func::<i32, i32>(&almacen, "run") {
        match run_v2.call(&mut almacen, valor_entrada) {
            Ok(retorno) => Ok(retorno),
            Err(error) => Err(match error.as_trap_code() {
                Some(TrapCode::OutOfFuel) => FallaApp::SinCombustible,
                Some(TrapCode::GrowthOperationLimited) => FallaApp::SinMemoria,
                _ => FallaApp::Trampa,
            }),
        }
    } else {
        let run_v1 = instancia
            .get_typed_func::<(), i32>(&almacen, "run")
            .map_err(|_| FallaApp::Carga)?;
        match run_v1.call(&mut almacen, ()) {
            Ok(retorno) => Ok(retorno),
            Err(error) => Err(match error.as_trap_code() {
                Some(TrapCode::OutOfFuel) => FallaApp::SinCombustible,
                Some(TrapCode::GrowthOperationLimited) => FallaApp::SinMemoria,
                _ => FallaApp::Trampa,
            }),
        }
    }
}

/// Reconciliacion del ciclo de vida. Cuando una `AplicacionWasm` muere —porque
/// fue desalojada y su tarea concluyo—, su canal de teclado debe darse de baja
/// de la difusion de la IRQ1. Sin esto, el manejador de interrupciones seguiria
/// empujando scancodes a una cola muerta: una fuga lenta pero segura.
impl Drop for AplicacionWasm {
    fn drop(&mut self) {
        let indice = self.almacen.data().indice_app;
        crate::async_system::teclado::cerrar_canal(indice);
        crate::async_system::puntero::cerrar_canal(indice);
        // Las sims que esta app dejo abiertas en el motor `tinkuy` mueren con
        // ella. Sin esto, un slot huerfano impediria que la proxima carga del
        // mismo indice obtuviera un slot fresco — y el bytecode del motor lo
        // mantendria vivo en su heap. Idempotente: si la app jamas pidio una
        // sim, la barrida es un no-op.
        crate::tinkuy::liberar_owner(indice);
        // MANIFIESTO DE MUERTE. Antes de soltar el `Store` —que devolveria
        // los bytes al heap del kernel sin tocarlos—, teñimos la memoria
        // lineal entera de ceros. El siguiente owner de esos bloques jamas
        // leera un byte de la app desalojada: ni un puntero, ni un texto, ni
        // una clave a medio borrar. Entropia cero al cerrar la jaula —el
        // bloque vuelve al kernel tan limpio como nacio—.
        self.memoria.data_mut(&mut self.almacen).fill(0);
    }
}
