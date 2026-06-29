//! llimphi-wasm-wasi — corre apps **WASI de consola** sobre wasmi.
//!
//! El catálogo no es trampa: además de las apps Tier 3 (UI, ABI propio
//! `wasm_view → WireNode`), el grueso del WASM público son **programas de línea
//! de comandos** compilados a `wasm32-wasi`: importan `wasi_snapshot_preview1`,
//! tienen un `_start` y escriben a stdout. Este crate los ejecuta: un shim
//! mínimo de WASI preview1 instancia el módulo, captura su stdout/stderr y
//! devuelve la salida + el código de salida. No es una GUI — es una consola,
//! que es exactamente lo que esos módulos son.
//!
//! Alcance honesto: se cubre el subconjunto que usa una herramienta CLI típica
//! (escribir, leer stdin, args, env, reloj, random, salir). Lo que toca
//! **filesystem o sockets reales** no está (esos imports no se enlazan: el
//! módulo que los use trap-ea al instanciar — la misma frontera física que el
//! resto del runtime). El component-model y las apps web (wasm-bindgen/DOM) son
//! otra historia: necesitan otro runtime / un navegador (eso es `puriy`).

use wasmi::{Caller, Engine, Linker, Module, Store};

/// Qué clase de módulo WASM es, mirando sus exports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmKind {
    /// App Tier 3 de Llimphi: exporta `wasm_view` (UI sobre el SDK).
    Tier3,
    /// Programa WASI de consola: exporta `_start`.
    WasiConsole,
    /// Ni una ni otra (no sabemos correrlo).
    Unknown,
}

/// Clasifica un `.wasm` por sus exports. Tier 3 manda (si exporta `wasm_view`
/// es una app de UI aunque también tuviera `_start`).
pub fn detect_kind(wasm: &[u8]) -> WasmKind {
    let engine = Engine::default();
    let module = match Module::new(&engine, wasm) {
        Ok(m) => m,
        Err(_) => return WasmKind::Unknown,
    };
    let mut tiene_view = false;
    let mut tiene_start = false;
    for exp in module.exports() {
        match exp.name() {
            "wasm_view" => tiene_view = true,
            "_start" => tiene_start = true,
            _ => {}
        }
    }
    if tiene_view {
        WasmKind::Tier3
    } else if tiene_start {
        WasmKind::WasiConsole
    } else {
        WasmKind::Unknown
    }
}

/// La salida de correr un programa de consola.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConsoleOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Código de salida (de `proc_exit`); `0` si el `_start` retornó normalmente.
    pub exit_code: i32,
}

impl ConsoleOutput {
    /// stdout como texto (lossy — no asumimos UTF-8 perfecto).
    pub fn stdout_text(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.stdout)
    }
    /// stderr como texto (lossy).
    pub fn stderr_text(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.stderr)
    }
}

/// Estado del proceso WASI que vive en el `Store` de wasmi.
struct WasiState {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    /// argv como bytes terminados en NUL (incluye argv[0]).
    args: Vec<Vec<u8>>,
    /// environ como `CLAVE=valor` en bytes.
    env: Vec<Vec<u8>>,
    /// stdin disponible para `fd_read`.
    stdin: Vec<u8>,
    stdin_pos: usize,
    /// Fijado por `proc_exit`; corta la ejecución vía trap.
    exit_code: Option<i32>,
    /// Semilla determinista para `random_get` (tests reproducibles).
    rng: u64,
}

// errno de WASI preview1 (los pocos que usamos).
const ERRNO_SUCCESS: i32 = 0;
const ERRNO_BADF: i32 = 8;
const ERRNO_INVAL: i32 = 28;

/// Mensaje del trap que usamos para señalar `proc_exit` (no es un error real).
const EXIT_TRAP: &str = "wasi:proc_exit";

/// Corre un módulo WASI de consola: instancia, llama `_start`, captura
/// stdout/stderr. `args` (sin contar argv[0], que se sintetiza) y `env`
/// (`("CLAVE","valor")`) y `stdin` alimentan al programa.
pub fn run_console(
    wasm: &[u8],
    args: &[String],
    env: &[(String, String)],
    stdin: &[u8],
) -> Result<ConsoleOutput, String> {
    let engine = Engine::default();
    let module = Module::new(&engine, wasm).map_err(|e| format!("compilar wasm: {e}"))?;

    let mut argv: Vec<Vec<u8>> = vec![b"app\0".to_vec()];
    for a in args {
        let mut v = a.clone().into_bytes();
        v.push(0);
        argv.push(v);
    }
    let environ: Vec<Vec<u8>> = env
        .iter()
        .map(|(k, val)| {
            let mut v = format!("{k}={val}").into_bytes();
            v.push(0);
            v
        })
        .collect();

    let state = WasiState {
        stdout: Vec::new(),
        stderr: Vec::new(),
        args: argv,
        env: environ,
        stdin: stdin.to_vec(),
        stdin_pos: 0,
        exit_code: None,
        rng: 0x9E37_79B9_7F4A_7C15,
    };
    let mut store = Store::new(&engine, state);
    let mut linker = Linker::<WasiState>::new(&engine);
    link_wasi(&mut linker)?;

    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .map_err(|e| format!("instanciar: {e}"))?;
    let start = instance
        .get_typed_func::<(), ()>(&store, "_start")
        .map_err(|e| format!("export `_start`: {e}"))?;

    // `_start` puede terminar por retorno normal o por el trap de `proc_exit`.
    let exit_code = match start.call(&mut store, ()) {
        Ok(()) => store.data().exit_code.unwrap_or(0),
        Err(e) => {
            // Si fue nuestro proc_exit, no es un error: el código está guardado.
            if let Some(code) = store.data().exit_code {
                code
            } else {
                let st = store.data();
                return Err(format!(
                    "trap en _start: {e} (stdout capturado: {})",
                    String::from_utf8_lossy(&st.stdout)
                ));
            }
        }
    };

    let st = store.data();
    Ok(ConsoleOutput {
        stdout: st.stdout.clone(),
        stderr: st.stderr.clone(),
        exit_code,
    })
}

/// Lee `len` bytes de la memoria del guest en un `Vec`.
fn mem_read(caller: &Caller<'_, WasiState>, ptr: i32, len: i32) -> Option<Vec<u8>> {
    let mem = caller.get_export("memory")?.into_memory()?;
    let mut buf = vec![0u8; len.max(0) as usize];
    mem.read(caller, ptr.max(0) as usize, &mut buf).ok()?;
    Some(buf)
}

/// Escribe `bytes` en la memoria del guest en `ptr`.
fn mem_write(caller: &mut Caller<'_, WasiState>, ptr: i32, bytes: &[u8]) -> bool {
    match caller.get_export("memory").and_then(|e| e.into_memory()) {
        Some(mem) => mem.write(caller, ptr.max(0) as usize, bytes).is_ok(),
        None => false,
    }
}

fn write_u32(caller: &mut Caller<'_, WasiState>, ptr: i32, v: u32) -> bool {
    mem_write(caller, ptr, &v.to_le_bytes())
}

/// Registra el shim de `wasi_snapshot_preview1`. Sólo el subconjunto de
/// consola: lo que no se enlaza, un módulo que lo importe lo descubre al
/// instanciar (no instancia) — frontera física.
fn link_wasi(linker: &mut Linker<WasiState>) -> Result<(), String> {
    const NS: &str = "wasi_snapshot_preview1";
    let err = |what: &str, e: wasmi::errors::LinkerError| format!("enlazar {what}: {e}");

    // fd_write(fd, iovs, iovs_len, nwritten) -> errno
    linker
        .func_wrap(
            NS,
            "fd_write",
            |mut caller: Caller<'_, WasiState>, fd: i32, iovs: i32, iovs_len: i32, nwritten: i32| -> i32 {
                let mut collected = Vec::new();
                for i in 0..iovs_len.max(0) {
                    let base = iovs + i * 8;
                    let hdr = match mem_read(&caller, base, 8) {
                        Some(h) => h,
                        None => return ERRNO_INVAL,
                    };
                    let ptr = u32::from_le_bytes(hdr[0..4].try_into().unwrap()) as i32;
                    let len = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as i32;
                    match mem_read(&caller, ptr, len) {
                        Some(d) => collected.extend_from_slice(&d),
                        None => return ERRNO_INVAL,
                    }
                }
                let total = collected.len() as u32;
                match fd {
                    1 => caller.data_mut().stdout.extend_from_slice(&collected),
                    2 => caller.data_mut().stderr.extend_from_slice(&collected),
                    _ => return ERRNO_BADF,
                }
                if !write_u32(&mut caller, nwritten, total) {
                    return ERRNO_INVAL;
                }
                ERRNO_SUCCESS
            },
        )
        .map_err(|e| err("fd_write", e))?;

    // fd_read(fd, iovs, iovs_len, nread) -> errno  (sólo fd 0 = stdin)
    linker
        .func_wrap(
            NS,
            "fd_read",
            |mut caller: Caller<'_, WasiState>, fd: i32, iovs: i32, iovs_len: i32, nread: i32| -> i32 {
                if fd != 0 {
                    return ERRNO_BADF;
                }
                let mut total = 0u32;
                for i in 0..iovs_len.max(0) {
                    let base = iovs + i * 8;
                    let hdr = match mem_read(&caller, base, 8) {
                        Some(h) => h,
                        None => return ERRNO_INVAL,
                    };
                    let ptr = u32::from_le_bytes(hdr[0..4].try_into().unwrap()) as i32;
                    let len = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as usize;
                    let (chunk, nuevo_pos) = {
                        let st = caller.data();
                        let restante = &st.stdin[st.stdin_pos.min(st.stdin.len())..];
                        let n = len.min(restante.len());
                        (restante[..n].to_vec(), st.stdin_pos + n)
                    };
                    if !chunk.is_empty() && !mem_write(&mut caller, ptr, &chunk) {
                        return ERRNO_INVAL;
                    }
                    caller.data_mut().stdin_pos = nuevo_pos;
                    total += chunk.len() as u32;
                    if chunk.is_empty() {
                        break;
                    }
                }
                if !write_u32(&mut caller, nread, total) {
                    return ERRNO_INVAL;
                }
                ERRNO_SUCCESS
            },
        )
        .map_err(|e| err("fd_read", e))?;

    // args_sizes_get(argc, buf_size) -> errno
    linker
        .func_wrap(
            NS,
            "args_sizes_get",
            |mut caller: Caller<'_, WasiState>, argc: i32, buf_size: i32| -> i32 {
                let (n, bytes) = {
                    let st = caller.data();
                    (st.args.len() as u32, st.args.iter().map(|a| a.len() as u32).sum::<u32>())
                };
                if !write_u32(&mut caller, argc, n) || !write_u32(&mut caller, buf_size, bytes) {
                    return ERRNO_INVAL;
                }
                ERRNO_SUCCESS
            },
        )
        .map_err(|e| err("args_sizes_get", e))?;

    // args_get(argv, buf) -> errno
    linker
        .func_wrap(
            NS,
            "args_get",
            |mut caller: Caller<'_, WasiState>, argv: i32, buf: i32| -> i32 {
                escribir_vector(&mut caller, argv, buf, |st| &st.args)
            },
        )
        .map_err(|e| err("args_get", e))?;

    // environ_sizes_get(count, buf_size) -> errno
    linker
        .func_wrap(
            NS,
            "environ_sizes_get",
            |mut caller: Caller<'_, WasiState>, count: i32, buf_size: i32| -> i32 {
                let (n, bytes) = {
                    let st = caller.data();
                    (st.env.len() as u32, st.env.iter().map(|a| a.len() as u32).sum::<u32>())
                };
                if !write_u32(&mut caller, count, n) || !write_u32(&mut caller, buf_size, bytes) {
                    return ERRNO_INVAL;
                }
                ERRNO_SUCCESS
            },
        )
        .map_err(|e| err("environ_sizes_get", e))?;

    // environ_get(environ, buf) -> errno
    linker
        .func_wrap(
            NS,
            "environ_get",
            |mut caller: Caller<'_, WasiState>, environ: i32, buf: i32| -> i32 {
                escribir_vector(&mut caller, environ, buf, |st| &st.env)
            },
        )
        .map_err(|e| err("environ_get", e))?;

    // clock_time_get(id, precision, time) -> errno  (reloj fijo: determinista)
    linker
        .func_wrap(
            NS,
            "clock_time_get",
            |mut caller: Caller<'_, WasiState>, _id: i32, _precision: i64, time: i32| -> i32 {
                if !mem_write(&mut caller, time, &0u64.to_le_bytes()) {
                    return ERRNO_INVAL;
                }
                ERRNO_SUCCESS
            },
        )
        .map_err(|e| err("clock_time_get", e))?;

    // random_get(buf, len) -> errno  (xorshift determinista)
    linker
        .func_wrap(
            NS,
            "random_get",
            |mut caller: Caller<'_, WasiState>, buf: i32, len: i32| -> i32 {
                let mut bytes = Vec::with_capacity(len.max(0) as usize);
                let mut x = caller.data().rng;
                for _ in 0..len.max(0) {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    bytes.push((x & 0xff) as u8);
                }
                caller.data_mut().rng = x;
                if !mem_write(&mut caller, buf, &bytes) {
                    return ERRNO_INVAL;
                }
                ERRNO_SUCCESS
            },
        )
        .map_err(|e| err("random_get", e))?;

    // proc_exit(code) -> ! : guarda el código y corta vía trap.
    linker
        .func_wrap(
            NS,
            "proc_exit",
            |mut caller: Caller<'_, WasiState>, code: i32| -> Result<(), wasmi::Error> {
                caller.data_mut().exit_code = Some(code);
                Err(wasmi::Error::host(ProcExit))
            },
        )
        .map_err(|e| err("proc_exit", e))?;

    // fd_close(fd) -> errno  (no-op para stdio)
    linker
        .func_wrap(NS, "fd_close", |_c: Caller<'_, WasiState>, _fd: i32| -> i32 { ERRNO_SUCCESS })
        .map_err(|e| err("fd_close", e))?;

    // fd_seek(fd, offset, whence, newoffset) -> errno  (stdio no busca: 0)
    linker
        .func_wrap(
            NS,
            "fd_seek",
            |mut caller: Caller<'_, WasiState>, _fd: i32, _off: i64, _whence: i32, newoffset: i32| -> i32 {
                let _ = mem_write(&mut caller, newoffset, &0u64.to_le_bytes());
                ERRNO_SUCCESS
            },
        )
        .map_err(|e| err("fd_seek", e))?;

    // fd_fdstat_get(fd, stat) -> errno  (stdio = char device; 24 bytes a cero+tipo)
    linker
        .func_wrap(
            NS,
            "fd_fdstat_get",
            |mut caller: Caller<'_, WasiState>, _fd: i32, stat: i32| -> i32 {
                // fs_filetype=2 (character_device); resto a cero.
                let mut buf = [0u8; 24];
                buf[0] = 2;
                if !mem_write(&mut caller, stat, &buf) {
                    return ERRNO_INVAL;
                }
                ERRNO_SUCCESS
            },
        )
        .map_err(|e| err("fd_fdstat_get", e))?;

    // fd_prestat_get(fd, prestat) -> errno  : no hay dirs preabiertos ⇒ BADF.
    // (wasi-libc itera hasta recibir BADF; devolverlo es lo correcto.)
    linker
        .func_wrap(NS, "fd_prestat_get", |_c: Caller<'_, WasiState>, _fd: i32, _p: i32| -> i32 {
            ERRNO_BADF
        })
        .map_err(|e| err("fd_prestat_get", e))?;

    // fd_prestat_dir_name(fd, path, path_len) -> errno : idem, BADF.
    linker
        .func_wrap(
            NS,
            "fd_prestat_dir_name",
            |_c: Caller<'_, WasiState>, _fd: i32, _p: i32, _l: i32| -> i32 { ERRNO_BADF },
        )
        .map_err(|e| err("fd_prestat_dir_name", e))?;

    Ok(())
}

/// Marcador de host-error que usamos para `proc_exit` (no es una falla real).
#[derive(Debug)]
struct ProcExit;
impl std::fmt::Display for ProcExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{EXIT_TRAP}")
    }
}
impl std::error::Error for ProcExit {}
impl wasmi::errors::HostError for ProcExit {}

/// Escribe un vector de cadenas estilo argv/environ: un array de punteros en
/// `ptr_array` y los bytes contiguos en `buf`. Patrón común a `args_get` y
/// `environ_get`.
fn escribir_vector(
    caller: &mut Caller<'_, WasiState>,
    ptr_array: i32,
    buf: i32,
    sel: impl Fn(&WasiState) -> &Vec<Vec<u8>>,
) -> i32 {
    let items: Vec<Vec<u8>> = sel(caller.data()).clone();
    let mut cursor = buf;
    for (i, item) in items.iter().enumerate() {
        // puntero i → posición actual del buffer
        if !write_u32(caller, ptr_array + (i as i32) * 4, cursor as u32) {
            return ERRNO_INVAL;
        }
        if !mem_write(caller, cursor, item) {
            return ERRNO_INVAL;
        }
        cursor += item.len() as i32;
    }
    ERRNO_SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compila WAT a wasm en el test (sin toolchain wasm32-wasi).
    fn wat(src: &str) -> Vec<u8> {
        wat::parse_str(src).expect("WAT válido")
    }

    /// Un módulo WASI mínimo: escribe `msg` a stdout vía fd_write y sale con
    /// `code` vía proc_exit. Arma el iovec en memoria en runtime.
    fn modulo_hola(msg: &str, code: i32) -> Vec<u8> {
        // Layout de memoria: [0..]=iovec(ptr=8,len), [8..]=msg.
        let bytes: String = msg.bytes().map(|b| format!("\\{b:02x}")).collect();
        let len = msg.len();
        wat(&format!(
            r#"(module
              (import "wasi_snapshot_preview1" "fd_write"
                (func $fd_write (param i32 i32 i32 i32) (result i32)))
              (import "wasi_snapshot_preview1" "proc_exit" (func $proc_exit (param i32)))
              (memory (export "memory") 1)
              (data (i32.const 8) "{bytes}")
              (func (export "_start")
                ;; iovec en 0: ptr=8, len={len}
                (i32.store (i32.const 0) (i32.const 8))
                (i32.store (i32.const 4) (i32.const {len}))
                ;; fd_write(1, iovs=0, iovs_len=1, nwritten=20)
                (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 20)))
                (call $proc_exit (i32.const {code}))
              )
            )"#
        ))
    }

    #[test]
    fn detect_kind_distingue_wasi_de_tier3() {
        let wasi = modulo_hola("x", 0);
        assert_eq!(detect_kind(&wasi), WasmKind::WasiConsole);
        // Un módulo que exporta wasm_view es Tier3.
        let tier3 = wat(r#"(module (func (export "wasm_view") (result i64) (i64.const 0)))"#);
        assert_eq!(detect_kind(&tier3), WasmKind::Tier3);
        // Sin ninguno de los dos: Unknown.
        let nada = wat(r#"(module (func (export "otra")))"#);
        assert_eq!(detect_kind(&nada), WasmKind::Unknown);
    }

    #[test]
    fn corre_y_captura_stdout_y_codigo() {
        let out = run_console(&modulo_hola("hola mundo\n", 0), &[], &[], &[]).expect("corre");
        assert_eq!(out.stdout_text(), "hola mundo\n");
        assert_eq!(out.exit_code, 0);
        assert!(out.stderr.is_empty());
    }

    #[test]
    fn proc_exit_propaga_codigo() {
        let out = run_console(&modulo_hola("x", 7), &[], &[], &[]).expect("corre");
        assert_eq!(out.exit_code, 7);
        assert_eq!(out.stdout_text(), "x");
    }

    #[test]
    fn args_y_environ_se_exponen() {
        // Un módulo que pide los tamaños de args y environ y los escribe a
        // memoria; comprobamos que el shim reporta los conteos correctos.
        let src = r#"(module
          (import "wasi_snapshot_preview1" "args_sizes_get"
            (func $args_sizes (param i32 i32) (result i32)))
          (import "wasi_snapshot_preview1" "environ_sizes_get"
            (func $env_sizes (param i32 i32) (result i32)))
          (import "wasi_snapshot_preview1" "fd_write"
            (func $fd_write (param i32 i32 i32 i32) (result i32)))
          (import "wasi_snapshot_preview1" "proc_exit" (func $proc_exit (param i32)))
          (memory (export "memory") 1)
          (func (export "_start")
            ;; args count → @0, buf → @4 ; environ count → @8, buf → @12
            (drop (call $args_sizes (i32.const 0) (i32.const 4)))
            (drop (call $env_sizes (i32.const 8) (i32.const 12)))
            ;; iovec @16 apuntando a @0, longitud 16 (los 4 u32) → stdout
            (i32.store (i32.const 16) (i32.const 0))
            (i32.store (i32.const 20) (i32.const 16))
            (drop (call $fd_write (i32.const 1) (i32.const 16) (i32.const 1) (i32.const 40)))
            (call $proc_exit (i32.const 0))
          ))"#;
        let out = run_console(
            &wat(src),
            &["uno".into(), "dos".into()],
            &[("K".into(), "v".into())],
            &[],
        )
        .expect("corre");
        // stdout = 4 u32 LE: argc, args_buf_size, env_count, env_buf_size.
        let w = |i: usize| u32::from_le_bytes(out.stdout[i * 4..i * 4 + 4].try_into().unwrap());
        // argc = argv0 ("app") + 2 = 3.
        assert_eq!(w(0), 3, "argc");
        assert_eq!(w(2), 1, "environ count = 1 (K=v)");
    }

    #[test]
    fn stdin_se_lee() {
        // Lee 5 bytes de stdin a memoria @100 y los reescribe a stdout.
        let src = r#"(module
          (import "wasi_snapshot_preview1" "fd_read"
            (func $fd_read (param i32 i32 i32 i32) (result i32)))
          (import "wasi_snapshot_preview1" "fd_write"
            (func $fd_write (param i32 i32 i32 i32) (result i32)))
          (import "wasi_snapshot_preview1" "proc_exit" (func $proc_exit (param i32)))
          (memory (export "memory") 1)
          (func (export "_start")
            ;; iovec @0: ptr=100, len=5
            (i32.store (i32.const 0) (i32.const 100))
            (i32.store (i32.const 4) (i32.const 5))
            (drop (call $fd_read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 8)))
            ;; reescribir lo leído (mismo iovec) a stdout
            (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 12)))
            (call $proc_exit (i32.const 0))
          ))"#;
        let out = run_console(&wat(src), &[], &[], b"hello world").expect("corre");
        assert_eq!(&out.stdout, b"hello");
    }
}
