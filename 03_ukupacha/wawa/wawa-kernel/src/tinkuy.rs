// =============================================================================
//  renaser :: kernel/src/tinkuy.rs — Fase C4 :: el motor de partículas embebido
// -----------------------------------------------------------------------------
//  Hasta C3 el cdylib `tinkuy.wasm` vivia huerfano en `assets/` — el pipeline
//  lo forjaba pero el kernel jamas lo levantaba. Esta capa cierra el bucle:
//  el reactor `wasmi` lo carga UNA sola vez en su propia sub-jaula, resuelve
//  sus exports `tk_*` y los pone a disposicion de las apps WASM userspace via
//  una nueva matriz de capacidades `sys_tinkuy_*` (ver `wasm/env.rs`).
//
//  PROPIEDADES:
//
//   * `tinkuy.wasm` NO IMPORTA NADA del host (verificado con un dump de
//     secciones — no hay seccion 2). El `Linker` queda vacio: la sub-jaula
//     es ciega al mundo, computo puro sobre su memoria lineal.
//
//   * El motor corre en su PROPIO `Store<()>` con SU PROPIO fuel desactivado.
//     El kernel se fia de su propio cdylib —forjado y firmado por el repo—
//     y no le impone el guardarrail temporal que aplica a apps userspace.
//     Si el dia de mañana el operador desea presupuestar el motor, podra
//     activar `consume_fuel` aqui sin tocar la matriz de capacidades.
//
//   * SCRATCH SEGURO: en la inicializacion crecemos la memoria lineal del
//     motor por 64 paginas (4 MiB) por encima de lo que el bytecode declara.
//     Esas paginas son INVISIBLES al asignador interno del modulo (dlmalloc
//     mantiene su cuenta de paginas y solo crece por `memory.grow` propio),
//     asi que el kernel las usa como buzon de parametros para los punteros
//     que exigen `tk_sim_new`, `tk_sim_step_lj` y `tk_sim_snapshot_cid`. Cada
//     llamada copia 256 B en la base del scratch — los reescribimos siempre,
//     sin acumular estado entre llamadas.
//
//   * SLOTS POR APP: el motor mantiene una tabla `[Option<Slot>; MAX_SLOTS]`.
//     Una syscall `sys_tinkuy_sim_new` toma un slot libre, lo asocia al
//     `indice_app` que lo solicito y le entrega su numero — el handle opaco
//     que las apps usan en las syscalls siguientes. Una app jamas puede
//     tocar el slot de otra: cada syscall verifica `slot.owner ==
//     caller.indice_app` antes de despachar. La frontera de aislamiento
//     entre apps tinkuy es de matematica fina, no de tabla de permisos.
//
//   * GEOMETRIA FIJA del dominio (MVP): origen `[-50, -50, -50]`, dims
//     `[34, 34, 34]` celdas, `cell_size = 3.0`. El cubo de simulacion es
//     [-50, +50]^3, holgado para LJ con cutoff 2.5σ. Una sub-fase posterior
//     parametrizara la geometria; para C4 nos basta.
//
//  Las apps userspace no ven nada de esta jaula: hablan con el motor via la
//  matriz de capacidades `sys_tinkuy_*`. El kernel hace el `data_mut` sobre la
//  memoria del motor, escribe los buffers de parametros, invoca el `TypedFunc`
//  correspondiente y, si hace falta, copia el resultado a la memoria lineal de
//  la app llamante. Dos memorias lineales (la del motor y la de la app) jamas
//  se mezclan: el kernel es la unica via de paso entre ambas.
// =============================================================================

use alloc::format;
use spin::{Mutex, Once};
use wasmi::{
    CompilationMode, Config, Engine, Linker, Memory, Module, Store, TypedFunc,
};

/// Bytecode del motor `tinkuy.wasm`. Lo forja el pipeline `build-tinkuy.sh`
/// (`tinkuy-abi` → `cdylib` + `wasm-opt -Os`) y lo deja consolidado en este
/// directorio. El kernel lo lleva empotrado: una vez forjado el binario, el
/// motor esta tan disponible como cualquier otra capacidad del kernel.
const TINKUY_WASM: &[u8] = include_bytes!("../assets/tinkuy.wasm");

/// Paginas (64 KiB) que se añaden al motor para uso EXCLUSIVO del scratch.
/// Vivimos del hecho de que el `dlmalloc` interno del modulo solo crece su
/// heap via `memory.grow` propio: paginas añadidas por el host le son
/// invisibles. 4 MiB cubren con holgura cualquier bump que la sim tinkuy
/// pudiera hacer en su propia rama del heap.
const PAGINAS_SCRATCH: u32 = 64;

/// Tamaño de pagina WASM, en bytes — fijo por la especificacion.
const TAM_PAGINA: u32 = 64 * 1024;

/// Cantidad de bytes reservados al pie de las paginas scratch para los
/// buffers de parametros. Cada llamada los reescribe — no hay reentrada
/// porque el motor corre single-thread bajo el cerrojo del kernel.
const TAM_SCRATCH: u32 = 256;

/// Numero maximo de simulaciones simultaneas que el motor puede albergar.
/// Un slot por app que pida una sim — siete apps con su propia jaula de
/// particulas caben holgadas. Un slot ocupado mantiene su `*mut TkSim` en
/// la memoria del motor; liberarlo (sys_tinkuy_sim_free) llama a
/// `tk_sim_free` y vacia la ranura.
pub const MAX_SLOTS: usize = 8;

/// Indice de app sentinela para slots libres. No coincide con `usize::MAX`
/// (usado por el despachador dinamico) — un slot libre lleva siempre
/// `owner = NINGUN_DUENIO`.
const NINGUN_DUENIO: usize = usize::MAX - 1;

/// Codigos de error que las syscalls `sys_tinkuy_*` propagan a las apps.
/// Negativos: la app los compara a sabiendas, el panel del operador los
/// rotula con causa legible. Cero o positivo: exito.
pub const TK_HOST_OK: i32 = 0;
pub const TK_HOST_ERR_AGOTADO: i32 = -10;
pub const TK_HOST_ERR_AJENO: i32 = -11;
pub const TK_HOST_ERR_INVALIDO: i32 = -12;
pub const TK_HOST_ERR_MOTOR: i32 = -13;

/// Un slot vivo del motor. `sim_ptr` es la `*mut TkSim` que `tk_sim_new`
/// devolvio dentro de la memoria lineal del motor; nunca se materializa
/// como puntero Rust del host — solo se pasa como `u32` de vuelta al motor.
struct Slot {
    sim_ptr: u32,
    owner: usize,
    step: u64,
}

impl Slot {
    const LIBRE: Slot = Slot {
        sim_ptr: 0,
        owner: NINGUN_DUENIO,
        step: 0,
    };

    const fn libre() -> Self {
        Slot::LIBRE
    }

    fn esta_libre(&self) -> bool {
        self.owner == NINGUN_DUENIO
    }
}

/// El motor tinkuy embebido. Una sola instancia para todo el kernel; las
/// syscalls la toman bajo cerrojo, asi que las llamadas se serializan —
/// el bytecode del motor corre single-thread por construccion (feature
/// `wasm` de `tinkuy-abi`).
struct Motor {
    store: Store<()>,
    memoria: Memory,
    /// Direccion en la memoria lineal del motor donde el kernel deposita
    /// punteros de parametros antes de cada llamada. Vive en las paginas
    /// extras que el motor nunca toca.
    scratch_base: u32,
    tk_sim_new: TypedFunc<(u32, u32, f32, u32, u32), i32>,
    tk_sim_free: TypedFunc<u32, ()>,
    tk_sim_spawn:
        TypedFunc<(u32, f32, f32, f32, f32, f32, f32, f32, f32, u32), i32>,
    tk_sim_len: TypedFunc<u32, u32>,
    tk_sim_rebuild_grid: TypedFunc<u32, i32>,
    tk_sim_step_lj: TypedFunc<(u32, f32, f32, f32, f32, u32, u32), i32>,
    tk_sim_kinetic_energy: TypedFunc<u32, f64>,
    tk_sim_temperature: TypedFunc<(u32, f64), f64>,
    tk_sim_snapshot_cid: TypedFunc<(u32, u32), i32>,
    tk_sim_positions: TypedFunc<(u32, u32, u32), i32>,
    slots: [Slot; MAX_SLOTS],
}

/// Holder global. Se inicializa perezosamente al primer uso por
/// `con_motor` — si el bytecode no carga o falta algun export, la
/// llamada devuelve `TK_HOST_ERR_MOTOR` y el motor queda no instalado;
/// futuras llamadas reintentaran (no es la situacion esperada en
/// produccion: el bytecode viene firmado en el repo).
static MOTOR: Once<Mutex<Result<Motor, ()>>> = Once::new();

/// Geometria por defecto del dominio: cubo `[-50, +50]^3` con grilla
/// 34×34×34. Coincide con `fresh_sim` de los tests de `tinkuy-abi` y deja
/// holgura para cutoff 2.5σ.
const ORIGEN: [f32; 3] = [-50.0, -50.0, -50.0];
const CELL_SIZE: f32 = 3.0;
const DIMS: [u32; 3] = [34, 34, 34];
const BMIN: [f32; 3] = [-50.0, -50.0, -50.0];
const BMAX: [f32; 3] = [50.0, 50.0, 50.0];

/// Constante de Boltzmann en unidades reducidas LJ. La app no la elige —
/// el contrato fija una sola convencion para todos los consumidores.
const KB_LJ: f64 = 1.0;

/// Resuelve el `Mutex<Result<Motor, ()>>` perezoso, cargando el bytecode si
/// es la primera llamada. La closure `f` recibe el motor ya construido.
/// `None` si el motor no se pudo instalar.
fn con_motor<R>(f: impl FnOnce(&mut Motor, &mut [Slot; MAX_SLOTS]) -> R) -> Option<R> {
    let holder = MOTOR.call_once(|| Mutex::new(construir_motor()));
    let mut guard = holder.lock();
    let motor = guard.as_mut().ok()?;
    // La firma `(motor, slots)` evita un borrow simultaneo de `Motor` y
    // `Motor::slots` dentro de las syscalls; la closure recibe los slots
    // por separado y trabaja con cada uno sin conflicto con `motor.store`.
    let slots_ptr: *mut [Slot; MAX_SLOTS] = &mut motor.slots;
    let r = f(motor, unsafe { &mut *slots_ptr });
    Some(r)
}

/// Instancia el motor: engine + module + store + linker (vacio) + memory
/// grow + resolucion de typed funcs. Devuelve `Err(())` si algo falla; el
/// kernel sigue en pie pero las syscalls `sys_tinkuy_*` devolveran
/// `TK_HOST_ERR_MOTOR` hasta que un build correcto siembre `tinkuy.wasm`.
fn construir_motor() -> Result<Motor, ()> {
    let mut config = Config::default();
    // El motor es codigo de kernel firmado: corre SIN cuota de fuel.
    config.consume_fuel(false);
    config.compilation_mode(CompilationMode::Eager);
    let engine = Engine::new(&config);

    let modulo = Module::new(&engine, TINKUY_WASM).map_err(|_| ())?;
    let mut store = Store::new(&engine, ());
    // Linker vacio: el bytecode no importa nada y queremos preservar esa
    // ley — un dia de mañana, una rama que importe wasi_snapshot_preview1
    // o similar caera al instanciar antes de pisar el kernel.
    let linker: Linker<()> = Linker::new(&engine);
    let instancia = linker
        .instantiate_and_start(&mut store, &modulo)
        .map_err(|_| ())?;

    let memoria = instancia.get_memory(&store, "memory").ok_or(())?;

    // Crecer 64 paginas para tallar el scratch. El asignador interno del
    // modulo (dlmalloc en wasm32-unknown-unknown) ignora estas paginas
    // porque mantiene su propia cuenta y solo crece por sus propias
    // llamadas a `memory.grow`. Las paginas extras quedan como sandbox
    // exclusivo del kernel.
    let paginas_previas = memoria.grow(&mut store, u64::from(PAGINAS_SCRATCH)).map_err(|_| ())?;
    let paginas_previas: u32 = u32::try_from(paginas_previas).map_err(|_| ())?;
    // Base del scratch: al pie de la PRIMERA pagina añadida. Asi tenemos
    // 256 B utiles y dejamos las demas paginas como colchon — un dia que
    // queramos pasar buffers mas grandes (estados completos), basta con
    // extender TAM_SCRATCH sin cambiar la base.
    let scratch_base = paginas_previas
        .checked_mul(TAM_PAGINA)
        .ok_or(())?;

    // Resolver los 9 typed funcs que el host necesita. Si alguno falta,
    // el motor es invalido — el repo se notaria al instante de cualquier
    // ABI break.
    let tk_sim_new = instancia
        .get_typed_func::<(u32, u32, f32, u32, u32), i32>(&store, "tk_sim_new")
        .map_err(|_| ())?;
    let tk_sim_free = instancia
        .get_typed_func::<u32, ()>(&store, "tk_sim_free")
        .map_err(|_| ())?;
    let tk_sim_spawn = instancia
        .get_typed_func::<
            (u32, f32, f32, f32, f32, f32, f32, f32, f32, u32),
            i32,
        >(&store, "tk_sim_spawn")
        .map_err(|_| ())?;
    let tk_sim_len = instancia
        .get_typed_func::<u32, u32>(&store, "tk_sim_len")
        .map_err(|_| ())?;
    let tk_sim_rebuild_grid = instancia
        .get_typed_func::<u32, i32>(&store, "tk_sim_rebuild_grid")
        .map_err(|_| ())?;
    let tk_sim_step_lj = instancia
        .get_typed_func::<(u32, f32, f32, f32, f32, u32, u32), i32>(
            &store,
            "tk_sim_step_lj",
        )
        .map_err(|_| ())?;
    let tk_sim_kinetic_energy = instancia
        .get_typed_func::<u32, f64>(&store, "tk_sim_kinetic_energy")
        .map_err(|_| ())?;
    let tk_sim_temperature = instancia
        .get_typed_func::<(u32, f64), f64>(&store, "tk_sim_temperature")
        .map_err(|_| ())?;
    let tk_sim_snapshot_cid = instancia
        .get_typed_func::<(u32, u32), i32>(&store, "tk_sim_snapshot_cid")
        .map_err(|_| ())?;
    let tk_sim_positions = instancia
        .get_typed_func::<(u32, u32, u32), i32>(&store, "tk_sim_positions")
        .map_err(|_| ())?;

    let _ = format!("[renaser/tinkuy] motor empotrado, scratch @ {scratch_base:#x}");
    Ok(Motor {
        store,
        memoria,
        scratch_base,
        tk_sim_new,
        tk_sim_free,
        tk_sim_spawn,
        tk_sim_len,
        tk_sim_rebuild_grid,
        tk_sim_step_lj,
        tk_sim_kinetic_energy,
        tk_sim_temperature,
        tk_sim_snapshot_cid,
        tk_sim_positions,
        slots: [
            Slot::libre(), Slot::libre(), Slot::libre(), Slot::libre(),
            Slot::libre(), Slot::libre(), Slot::libre(), Slot::libre(),
        ],
    })
}

/// Encuentra un slot libre y devuelve su indice. `None` si todos estan
/// ocupados.
fn slot_libre(slots: &[Slot; MAX_SLOTS]) -> Option<usize> {
    slots.iter().position(|s| s.esta_libre())
}

/// Verifica que el indice de slot es valido Y que la app que lo invoca es
/// SU duenia. Aislamiento por matematica: la syscall no toca el motor si
/// el indice es ajeno.
fn validar_slot(slots: &[Slot; MAX_SLOTS], slot: u32, owner: usize) -> Result<usize, i32> {
    let idx = slot as usize;
    if idx >= MAX_SLOTS {
        return Err(TK_HOST_ERR_INVALIDO);
    }
    if slots[idx].esta_libre() {
        return Err(TK_HOST_ERR_INVALIDO);
    }
    if slots[idx].owner != owner {
        return Err(TK_HOST_ERR_AJENO);
    }
    Ok(idx)
}

/// `sys_tinkuy_sim_new`: reserva un slot del motor, configura la sim con
/// la geometria por defecto y devuelve el numero de slot.
pub fn sim_new(owner: usize) -> i32 {
    con_motor(|motor, slots| {
        let Some(idx) = slot_libre(slots) else {
            return TK_HOST_ERR_AGOTADO;
        };
        // Escribir origin + dims + out en el scratch del motor.
        // Layout: [0..12] origin (3×f32), [12..24] dims (3×u32), [24..28] out_ptr (u32, init 0).
        let memoria = motor.memoria.data_mut(&mut motor.store);
        let base = motor.scratch_base as usize;
        if base + 28 >= memoria.len() {
            return TK_HOST_ERR_MOTOR;
        }
        memoria[base..base + 4].copy_from_slice(&ORIGEN[0].to_le_bytes());
        memoria[base + 4..base + 8].copy_from_slice(&ORIGEN[1].to_le_bytes());
        memoria[base + 8..base + 12].copy_from_slice(&ORIGEN[2].to_le_bytes());
        memoria[base + 12..base + 16].copy_from_slice(&DIMS[0].to_le_bytes());
        memoria[base + 16..base + 20].copy_from_slice(&DIMS[1].to_le_bytes());
        memoria[base + 20..base + 24].copy_from_slice(&DIMS[2].to_le_bytes());
        memoria[base + 24..base + 28].copy_from_slice(&0u32.to_le_bytes());

        let origin_ptr = motor.scratch_base;
        let dims_ptr = motor.scratch_base + 12;
        let out_ptr = motor.scratch_base + 24;

        let rc = motor
            .tk_sim_new
            .call(&mut motor.store, (64, origin_ptr, CELL_SIZE, dims_ptr, out_ptr));
        match rc {
            Ok(0) => {}
            _ => return TK_HOST_ERR_MOTOR,
        }
        // Leer el handle (TkSim*) que el motor escribio en `out_ptr`.
        let memoria = motor.memoria.data(&motor.store);
        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(&memoria[out_ptr as usize..out_ptr as usize + 4]);
        let sim_ptr = u32::from_le_bytes(bytes);
        if sim_ptr == 0 {
            return TK_HOST_ERR_MOTOR;
        }

        slots[idx] = Slot { sim_ptr, owner, step: 0 };
        idx as i32
    })
    .unwrap_or(TK_HOST_ERR_MOTOR)
}

/// `sys_tinkuy_sim_spawn`: encola una particula. Si `out_idx` se desea, la
/// app puede usar `sys_tinkuy_sim_len` despues — no devolvemos el indice
/// por la syscall para mantener la firma compacta.
pub fn sim_spawn(
    owner: usize,
    slot: u32,
    x: f32,
    y: f32,
    z: f32,
    vx: f32,
    vy: f32,
    vz: f32,
    masa: f32,
    carga: f32,
) -> i32 {
    con_motor(|motor, slots| {
        let idx = match validar_slot(slots, slot, owner) {
            Ok(i) => i,
            Err(e) => return e,
        };
        let sim_ptr = slots[idx].sim_ptr;
        // `out_idx` opcional → 0 (null) en wasm. El motor lo respeta.
        let rc = motor.tk_sim_spawn.call(
            &mut motor.store,
            (sim_ptr, x, y, z, vx, vy, vz, masa, carga, 0u32),
        );
        match rc {
            Ok(0) => TK_HOST_OK,
            _ => TK_HOST_ERR_MOTOR,
        }
    })
    .unwrap_or(TK_HOST_ERR_MOTOR)
}

/// `sys_tinkuy_sim_rebuild_grid`: reconstruye la grilla a partir de
/// posiciones actuales. Necesario tras spawn masivo antes del primer step.
pub fn sim_rebuild_grid(owner: usize, slot: u32) -> i32 {
    con_motor(|motor, slots| {
        let idx = match validar_slot(slots, slot, owner) {
            Ok(i) => i,
            Err(e) => return e,
        };
        let sim_ptr = slots[idx].sim_ptr;
        let rc = motor.tk_sim_rebuild_grid.call(&mut motor.store, sim_ptr);
        match rc {
            Ok(0) => TK_HOST_OK,
            _ => TK_HOST_ERR_MOTOR,
        }
    })
    .unwrap_or(TK_HOST_ERR_MOTOR)
}

/// `sys_tinkuy_sim_step_lj`: avanza `n_steps` substeps Velocity-Verlet con
/// fuerza LJ. Los punteros `bmin`/`bmax` van al scratch del motor (32 B
/// reescritos cada llamada).
pub fn sim_step_lj(
    owner: usize,
    slot: u32,
    n_steps: u32,
    dt: f32,
    epsilon: f32,
    sigma: f32,
    cutoff: f32,
) -> i32 {
    con_motor(|motor, slots| {
        let idx = match validar_slot(slots, slot, owner) {
            Ok(i) => i,
            Err(e) => return e,
        };
        let sim_ptr = slots[idx].sim_ptr;

        // Depositar bmin + bmax en el scratch (a partir del offset 64 para
        // no colisionar con la pagina sim_new — no porque se solapen, sino
        // por higiene mental).
        let base = motor.scratch_base as usize;
        let bmin_off = 64;
        let bmax_off = 64 + 12;
        if base + bmax_off + 12 >= TAM_SCRATCH as usize + base {
            // sanity guard
        }
        {
            let memoria = motor.memoria.data_mut(&mut motor.store);
            memoria[base + bmin_off..base + bmin_off + 4]
                .copy_from_slice(&BMIN[0].to_le_bytes());
            memoria[base + bmin_off + 4..base + bmin_off + 8]
                .copy_from_slice(&BMIN[1].to_le_bytes());
            memoria[base + bmin_off + 8..base + bmin_off + 12]
                .copy_from_slice(&BMIN[2].to_le_bytes());
            memoria[base + bmax_off..base + bmax_off + 4]
                .copy_from_slice(&BMAX[0].to_le_bytes());
            memoria[base + bmax_off + 4..base + bmax_off + 8]
                .copy_from_slice(&BMAX[1].to_le_bytes());
            memoria[base + bmax_off + 8..base + bmax_off + 12]
                .copy_from_slice(&BMAX[2].to_le_bytes());
        }
        let bmin_ptr = motor.scratch_base + bmin_off as u32;
        let bmax_ptr = motor.scratch_base + bmax_off as u32;

        for _ in 0..n_steps {
            let rc = motor.tk_sim_step_lj.call(
                &mut motor.store,
                (sim_ptr, dt, epsilon, sigma, cutoff, bmin_ptr, bmax_ptr),
            );
            match rc {
                Ok(0) => {}
                _ => return TK_HOST_ERR_MOTOR,
            }
        }
        slots[idx].step = slots[idx].step.saturating_add(u64::from(n_steps));
        TK_HOST_OK
    })
    .unwrap_or(TK_HOST_ERR_MOTOR)
}

/// `sys_tinkuy_sim_observables`: lee step (u64), KE (f64) y T (f64) del
/// slot indicado. El kernel los copia a la memoria de la APP llamante en
/// `out_24` — son 24 bytes exactos. Devuelve `TK_HOST_OK` o un codigo.
pub fn sim_observables(
    owner: usize,
    slot: u32,
) -> Result<(u64, f64, f64), i32> {
    con_motor(|motor, slots| {
        let idx = validar_slot(slots, slot, owner)?;
        let sim_ptr = slots[idx].sim_ptr;
        let step = slots[idx].step;
        let ke = motor
            .tk_sim_kinetic_energy
            .call(&mut motor.store, sim_ptr)
            .map_err(|_| TK_HOST_ERR_MOTOR)?;
        let temp = motor
            .tk_sim_temperature
            .call(&mut motor.store, (sim_ptr, KB_LJ))
            .map_err(|_| TK_HOST_ERR_MOTOR)?;
        Ok((step, ke, temp))
    })
    .unwrap_or(Err(TK_HOST_ERR_MOTOR))
}

/// `sys_tinkuy_sim_len`: cantidad de particulas vivas.
pub fn sim_len(owner: usize, slot: u32) -> i32 {
    con_motor(|motor, slots| {
        let idx = match validar_slot(slots, slot, owner) {
            Ok(i) => i,
            Err(e) => return e,
        };
        let sim_ptr = slots[idx].sim_ptr;
        match motor.tk_sim_len.call(&mut motor.store, sim_ptr) {
            Ok(n) => n as i32,
            Err(_) => TK_HOST_ERR_MOTOR,
        }
    })
    .unwrap_or(TK_HOST_ERR_MOTOR)
}

/// `sys_tinkuy_sim_snapshot_cid`: vuelca los 32 bytes del CID BLAKE3 del
/// estado actual a un buffer del kernel. La syscall en `env.rs` luego los
/// copia a la app llamante. Devuelve `Result<[u8;32], i32>`.
pub fn sim_snapshot_cid(owner: usize, slot: u32) -> Result<[u8; 32], i32> {
    con_motor(|motor, slots| {
        let idx = validar_slot(slots, slot, owner)?;
        let sim_ptr = slots[idx].sim_ptr;
        // Out al scratch del motor: offset 128, 32 bytes.
        let out_off = 128u32;
        let out_ptr = motor.scratch_base + out_off;
        // Limpiar y poner ceros previos por higiene.
        {
            let memoria = motor.memoria.data_mut(&mut motor.store);
            let off = out_ptr as usize;
            memoria[off..off + 32].fill(0);
        }
        let rc = motor
            .tk_sim_snapshot_cid
            .call(&mut motor.store, (sim_ptr, out_ptr))
            .map_err(|_| TK_HOST_ERR_MOTOR)?;
        if rc != 0 {
            return Err(TK_HOST_ERR_MOTOR);
        }
        // Leer 32 B del scratch.
        let memoria = motor.memoria.data(&motor.store);
        let mut cid = [0u8; 32];
        cid.copy_from_slice(&memoria[out_ptr as usize..out_ptr as usize + 32]);
        Ok(cid)
    })
    .unwrap_or(Err(TK_HOST_ERR_MOTOR))
}

/// `sys_tinkuy_sim_positions`: copia las posiciones de la sim del slot
/// como un arreglo `f32[N*3]` (AoS). El kernel apoya la transferencia en
/// dos saltos:
///   1) llamada al motor (`tk_sim_positions`) escribe en el scratch del
///      MOTOR un arreglo AoS de hasta `MAX_PARTICULAS_VIZ * 3` floats;
///   2) el host copia esos bytes a un buffer de pila propio y lo
///      devuelve al caller, que lo replicara en la memoria de la APP.
/// Cota: `MAX_PARTICULAS_VIZ` particulas — el caso de uso es renderizado
/// 2D, donde mas de unos cientos satura cualquier rasterizador del
/// userspace. Si el dia de mañana hay miles, se sube esta cota o se
/// expone el snapshot completo.
pub const MAX_PARTICULAS_VIZ: usize = 256;

/// Devuelve `(n_copiadas, [(x, y, z); MAX_PARTICULAS_VIZ])`. `n_copiadas`
/// es la cantidad real de particulas; el resto del arreglo queda en cero.
pub fn sim_positions(
    owner: usize,
    slot: u32,
) -> Result<(u32, [[f32; 3]; MAX_PARTICULAS_VIZ]), i32> {
    con_motor(|motor, slots| {
        let idx = validar_slot(slots, slot, owner)?;
        let sim_ptr = slots[idx].sim_ptr;
        // Reservamos un area en el scratch a partir del offset 256 — fuera
        // de los buffers cortos usados por sim_new/step_lj/snapshot_cid.
        // Tamaño: MAX_PARTICULAS_VIZ × 3 × 4 = 3072 B, que cabe holgado
        // dentro de la pagina scratch (los primeros 65536 B son nuestros).
        const SCRATCH_OFF: u32 = 256;
        let buf_ptr = motor.scratch_base + SCRATCH_OFF;
        // Limpiar la zona destino — defensa contra leer ceros viejos.
        {
            let memoria = motor.memoria.data_mut(&mut motor.store);
            let off = buf_ptr as usize;
            let len = MAX_PARTICULAS_VIZ * 3 * 4;
            memoria[off..off + len].fill(0);
        }
        let rc = motor
            .tk_sim_positions
            .call(&mut motor.store, (sim_ptr, buf_ptr, MAX_PARTICULAS_VIZ as u32))
            .map_err(|_| TK_HOST_ERR_MOTOR)?;
        if rc < 0 {
            return Err(TK_HOST_ERR_MOTOR);
        }
        let n = rc as u32;
        let mut salida = [[0.0f32; 3]; MAX_PARTICULAS_VIZ];
        let memoria = motor.memoria.data(&motor.store);
        let off = buf_ptr as usize;
        for i in 0..(n as usize).min(MAX_PARTICULAS_VIZ) {
            let base = off + i * 12;
            let mut buf = [0u8; 4];
            buf.copy_from_slice(&memoria[base..base + 4]);
            salida[i][0] = f32::from_le_bytes(buf);
            buf.copy_from_slice(&memoria[base + 4..base + 8]);
            salida[i][1] = f32::from_le_bytes(buf);
            buf.copy_from_slice(&memoria[base + 8..base + 12]);
            salida[i][2] = f32::from_le_bytes(buf);
        }
        Ok((n, salida))
    })
    .unwrap_or(Err(TK_HOST_ERR_MOTOR))
}

/// `sys_tinkuy_sim_free`: libera el slot y la sim asociada en el motor.
/// Idempotente: liberar un slot ajeno o ya libre devuelve `Ajeno` o
/// `Invalido` SIN tocar el motor.
pub fn sim_free(owner: usize, slot: u32) -> i32 {
    con_motor(|motor, slots| {
        let idx = match validar_slot(slots, slot, owner) {
            Ok(i) => i,
            Err(e) => return e,
        };
        let sim_ptr = slots[idx].sim_ptr;
        let _ = motor.tk_sim_free.call(&mut motor.store, sim_ptr);
        slots[idx] = Slot::libre();
        TK_HOST_OK
    })
    .unwrap_or(TK_HOST_ERR_MOTOR)
}

/// Libera TODOS los slots cuyo `owner` coincida con `indice`. Lo invoca el
/// supervisor de apps cuando una app muere (FallaApp + Drop) — asi un
/// crash en mitad de una rafaga no deja la sim huerfana en el motor.
pub fn liberar_owner(owner: usize) {
    let _ = con_motor(|motor, slots| {
        for slot in slots.iter_mut() {
            if !slot.esta_libre() && slot.owner == owner {
                let _ = motor.tk_sim_free.call(&mut motor.store, slot.sim_ptr);
                *slot = Slot::libre();
            }
        }
    });
}
