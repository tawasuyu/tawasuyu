use super::*;

pub(crate) fn enlazar_anuncio_tinkuy(
    enlazador: &mut Linker<ContextoCapacidades>,
    permisos: Permisos,
) -> Result<(), Error> {
    // --- CAPACIDAD pasiva :: sys_canal_anuncio(salida, capacidad) -> i32 ---
    // Fase 64 :: vuelca el ULTIMO `AnunciarCanal` recibido por Akasha a la
    // memoria de la app, en un layout fijo de 168 B —idéntico al `anuncio.bin`
    // que produce `agora-cli wawa publicar`—:
    //
    //   canal(32) | raiz(32) | autor(32) | timestamp_le(8) | firma(64)
    //
    // Retorno: 168 si habia un anuncio (bytes escritos), 0 si la ranura esta
    // vacia, `CapacidadInsuficiente` si `capacidad` < 168. Lectura PASIVA: es
    // dato publico de red, no muta nada; la app `mudanza` la sondea cada
    // fotograma para descubrir propuestas. La decision de aceptar (y la
    // verificacion soberana) viven en `sys_canal_aceptar`, gateada por RAIZ.
    enlazador.func_wrap(
        "renaser",
        "sys_canal_anuncio",
        |mut caller: Caller<'_, ContextoCapacidades>,
         salida: u32,
         capacidad: u32|
         -> Result<i32, Error> {
            const LARGO: usize = 32 + 32 + 32 + 8 + 64; // 168
            let Some(anuncio) = crate::akasha::ultimo_anuncio() else {
                return Ok(0);
            };
            if (capacidad as usize) < LARGO {
                return Ok(CodigoError::CapacidadInsuficiente.como_i32());
            }
            let mut buf = [0u8; LARGO];
            buf[0..32].copy_from_slice(&anuncio.canal);
            buf[32..64].copy_from_slice(&anuncio.raiz);
            buf[64..96].copy_from_slice(&anuncio.autor);
            buf[96..104].copy_from_slice(&anuncio.timestamp.to_le_bytes());
            buf[104..168].copy_from_slice(&anuncio.firma);
            let memoria = obtener_memoria(&caller)?;
            {
                let m = memoria.data(&caller);
                rango(
                    m,
                    salida,
                    LARGO,
                    "WASM :: sys_canal_anuncio desbordo la memoria lineal",
                )?;
            }
            let m = memoria.data_mut(&mut caller);
            m[salida as usize..salida as usize + LARGO].copy_from_slice(&buf);
            Ok(LARGO as i32)
        },
    )?;

    // =========================================================================
    //  FASE C4 :: motor `tinkuy` empotrado
    // -------------------------------------------------------------------------
    //  Las apps con PERMISO_TINKUY reciben acceso al motor de particulas del
    //  kernel — una sub-jaula `wasmi` aparte que carga `assets/tinkuy.wasm` y
    //  resuelve sus exports `tk_*`. Cada syscall delega al modulo
    //  `crate::tinkuy`, que toma el cerrojo del motor, hace `data_mut` sobre
    //  SU memoria lineal (no la de la app), llama al `TypedFunc` y, si hace
    //  falta, copia el resultado a la memoria lineal de la app llamante con
    //  los limites verificados. Dos memorias jamas se mezclan.
    //
    //  Las syscalls comparten contrato:
    //    * Toman un `slot: u32` que la app obtuvo de `sys_tinkuy_sim_new`.
    //    * Verifican que el slot pertenezca a SU `indice_app` — el aislamiento
    //      entre apps tinkuy es matematica, no tabla de permisos.
    //    * Devuelven `TK_HOST_OK = 0`, valores positivos especificos (slot
    //      asignado, len) o negativos (`Agotado`, `Ajeno`, `Invalido`,
    //      `Motor` — codigos de `crate::tinkuy`).
    // =========================================================================
    if permisos & PERMISO_TINKUY != 0 {
        // --- CAPACIDAD :: sys_tinkuy_sim_new() -> i32 ---
        // Reserva una sim con geometria fija (cubo [-50, +50]^3, grid 34^3,
        // cell_size 3.0). Devuelve el indice de slot (>=0) o un error.
        enlazador.func_wrap(
            "renaser",
            "sys_tinkuy_sim_new",
            |caller: Caller<'_, ContextoCapacidades>| -> i32 {
                let owner = caller.data().indice_app;
                crate::tinkuy::sim_new(owner)
            },
        )?;

        // --- CAPACIDAD :: sys_tinkuy_sim_spawn(slot, x,y,z, vx,vy,vz, m, q) -> i32 ---
        // Añade una particula a la sim del slot. Codifica los nueve f32 como
        // tipos WASM nativos — sin punteros, sin scratch.
        enlazador.func_wrap(
            "renaser",
            "sys_tinkuy_sim_spawn",
            |caller: Caller<'_, ContextoCapacidades>,
             slot: u32,
             x: f32,
             y: f32,
             z: f32,
             vx: f32,
             vy: f32,
             vz: f32,
             masa: f32,
             carga: f32|
             -> i32 {
                let owner = caller.data().indice_app;
                crate::tinkuy::sim_spawn(owner, slot, x, y, z, vx, vy, vz, masa, carga)
            },
        )?;

        // --- CAPACIDAD :: sys_tinkuy_sim_rebuild_grid(slot) -> i32 ---
        // Reconstruye la grilla espacial. Llamada obligada despues de spawn
        // masivo y antes del primer `step_lj`.
        enlazador.func_wrap(
            "renaser",
            "sys_tinkuy_sim_rebuild_grid",
            |caller: Caller<'_, ContextoCapacidades>, slot: u32| -> i32 {
                let owner = caller.data().indice_app;
                crate::tinkuy::sim_rebuild_grid(owner, slot)
            },
        )?;

        // --- CAPACIDAD :: sys_tinkuy_sim_step_lj(slot, n_steps, dt, eps, sigma, cutoff) -> i32 ---
        // Avanza `n_steps` substeps de Velocity-Verlet con fuerza LJ. Los
        // bmin/bmax los fija el motor (mismo cubo de `sim_new`).
        enlazador.func_wrap(
            "renaser",
            "sys_tinkuy_sim_step_lj",
            |caller: Caller<'_, ContextoCapacidades>,
             slot: u32,
             n_steps: u32,
             dt: f32,
             eps: f32,
             sigma: f32,
             cutoff: f32|
             -> i32 {
                let owner = caller.data().indice_app;
                crate::tinkuy::sim_step_lj(owner, slot, n_steps, dt, eps, sigma, cutoff)
            },
        )?;

        // --- CAPACIDAD :: sys_tinkuy_sim_len(slot) -> i32 ---
        // Particulas vivas en la sim.
        enlazador.func_wrap(
            "renaser",
            "sys_tinkuy_sim_len",
            |caller: Caller<'_, ContextoCapacidades>, slot: u32| -> i32 {
                let owner = caller.data().indice_app;
                crate::tinkuy::sim_len(owner, slot)
            },
        )?;

        // --- CAPACIDAD :: sys_tinkuy_sim_observables(slot, out_24_ptr) -> i32 ---
        // Escribe 24 bytes en la memoria de la app: step (u64 LE, 8 B) +
        // KE (f64 LE, 8 B) + T (f64 LE, 8 B). Las apps lo leen plano sin
        // depender de la crate `format`. Limites verificados a fondo.
        enlazador.func_wrap(
            "renaser",
            "sys_tinkuy_sim_observables",
            |mut caller: Caller<'_, ContextoCapacidades>,
             slot: u32,
             out_24_ptr: u32|
             -> Result<i32, Error> {
                let owner = caller.data().indice_app;
                let (step, ke, temp) = match crate::tinkuy::sim_observables(owner, slot) {
                    Ok(t) => t,
                    Err(codigo) => return Ok(codigo),
                };
                let memoria = obtener_memoria(&caller)?;
                {
                    let m = memoria.data(&caller);
                    rango(
                        m,
                        out_24_ptr,
                        24,
                        "WASM :: sys_tinkuy_sim_observables desbordo memoria",
                    )?;
                }
                let m = memoria.data_mut(&mut caller);
                let off = out_24_ptr as usize;
                m[off..off + 8].copy_from_slice(&step.to_le_bytes());
                m[off + 8..off + 16].copy_from_slice(&ke.to_le_bytes());
                m[off + 16..off + 24].copy_from_slice(&temp.to_le_bytes());
                Ok(crate::tinkuy::TK_HOST_OK)
            },
        )?;

        // --- CAPACIDAD :: sys_tinkuy_sim_snapshot_cid(slot, out_32_ptr) -> i32 ---
        // Escribe 32 bytes del CID BLAKE3 del estado actual en la memoria
        // de la app. El kernel lo obtiene del motor en su scratch y lo
        // copia con limites firmes.
        enlazador.func_wrap(
            "renaser",
            "sys_tinkuy_sim_snapshot_cid",
            |mut caller: Caller<'_, ContextoCapacidades>,
             slot: u32,
             out_32_ptr: u32|
             -> Result<i32, Error> {
                let owner = caller.data().indice_app;
                let cid = match crate::tinkuy::sim_snapshot_cid(owner, slot) {
                    Ok(c) => c,
                    Err(codigo) => return Ok(codigo),
                };
                let memoria = obtener_memoria(&caller)?;
                {
                    let m = memoria.data(&caller);
                    rango(
                        m,
                        out_32_ptr,
                        32,
                        "WASM :: sys_tinkuy_sim_snapshot_cid desbordo memoria",
                    )?;
                }
                let m = memoria.data_mut(&mut caller);
                m[out_32_ptr as usize..out_32_ptr as usize + 32].copy_from_slice(&cid);
                Ok(crate::tinkuy::TK_HOST_OK)
            },
        )?;

        // --- CAPACIDAD :: sys_tinkuy_sim_free(slot) -> i32 ---
        // Libera el slot y la sim. Idempotente sobre slots libres/ajenos.
        enlazador.func_wrap(
            "renaser",
            "sys_tinkuy_sim_free",
            |caller: Caller<'_, ContextoCapacidades>, slot: u32| -> i32 {
                let owner = caller.data().indice_app;
                crate::tinkuy::sim_free(owner, slot)
            },
        )?;

        // --- CAPACIDAD :: sys_tinkuy_sim_positions(slot, out_ptr, cap_count) -> i32 ---
        // Copia las posiciones (x,y,z) en AoS hacia la memoria lineal de la
        // app. `cap_count` es el numero de PARTICULAS que cabe en `out_ptr`;
        // la syscall valida que `cap_count * 12` esta dentro de la memoria
        // de la app y devuelve la cantidad real copiada (>= 0) o un codigo
        // negativo. Cota dura: `MAX_PARTICULAS_VIZ` (256) — el kernel
        // truncara silenciosamente si la sim tuviera mas.
        enlazador.func_wrap(
            "renaser",
            "sys_tinkuy_sim_positions",
            |mut caller: Caller<'_, ContextoCapacidades>,
             slot: u32,
             out_ptr: u32,
             cap_count: u32|
             -> Result<i32, Error> {
                let owner = caller.data().indice_app;
                let (n, posiciones) = match crate::tinkuy::sim_positions(owner, slot) {
                    Ok(t) => t,
                    Err(codigo) => return Ok(codigo),
                };
                let n_a_copiar = n.min(cap_count).min(
                    crate::tinkuy::MAX_PARTICULAS_VIZ as u32,
                ) as usize;
                let bytes_a_copiar = n_a_copiar * 12;
                let memoria = obtener_memoria(&caller)?;
                {
                    let m = memoria.data(&caller);
                    rango(
                        m,
                        out_ptr,
                        bytes_a_copiar,
                        "WASM :: sys_tinkuy_sim_positions desbordo memoria",
                    )?;
                }
                let m = memoria.data_mut(&mut caller);
                let off = out_ptr as usize;
                for i in 0..n_a_copiar {
                    let base = off + i * 12;
                    m[base..base + 4]
                        .copy_from_slice(&posiciones[i][0].to_le_bytes());
                    m[base + 4..base + 8]
                        .copy_from_slice(&posiciones[i][1].to_le_bytes());
                    m[base + 8..base + 12]
                        .copy_from_slice(&posiciones[i][2].to_le_bytes());
                }
                Ok(n_a_copiar as i32)
            },
        )?;
    } // PERMISO_TINKUY
    Ok(())
}

