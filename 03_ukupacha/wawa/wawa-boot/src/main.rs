// =============================================================================
//  renaser :: boot/src/main.rs — Fase 1.5 :: el puente hacia el silicio
// -----------------------------------------------------------------------------
//  Un kernel bare-metal no nace solo: alguien debe fusionarlo con un cargador,
//  sellarlo en una imagen de disco arrancable y entregarlo al hardware. Esa es
//  la mision de este orquestador de ANFITRION.
//
//  Desde la Fase 7b hace algo mas: SIEMBRA el grafo. El kernel ya no empotra
//  el userspace —ni un solo `include_bytes!` de un `.wasm`—; en su lugar, este
//  constructor pre-puebla el disco de objetos con el bytecode de las apps de
//  genesis y el Manifiesto de Genesis que dicta cuales arrancan, en que region
//  y con que cuota. Para ello habla el MISMO format del grafo que el kernel,
//  a traves de la crate compartida `format`.
//
//  El flujo es deliberadamente lineal y sin ambiguedad:
//
//    1. Localizar el ELF nativo del kernel (lo inyecta la dep. de artefacto).
//    2. Fusionarlo con el cargador UEFI en una imagen de disco GPT.
//    3. Sembrar el disco de objetos: el grafo poblado con el bytecode del
//       userspace y el Manifiesto de Genesis (Fase 7b).
//    4. Lanzar QEMU con la imagen, el disco de objetos y el firmware OVMF.
//
//  Cada paso que pueda fallar lo hace en voz alta, con un mensaje accionable:
//  preferimos un error claro a un arranque silencioso hacia la nada.
// =============================================================================

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

// El format del grafo de objetos en disco — el MISMO nucleo `no_std` que
// enlaza el kernel. Gracias a el, lo que `boot` siembra y lo que el kernel lee
// es, byte a byte, el mismo idioma.
use format::{
    ConcesionCapacidad, EntradaApp, Hash, Manifiesto, Objeto, SuperBloque, MAGIA, MAX_OBJETO,
    TAM_SECTOR, VERSION_MANIFIESTO, VERSION_SUPERBLOQUE,
};

/// Ruta del ELF del kernel, ya compilado para `x86_64-unknown-none`.
///
/// La dependencia de artefacto define esta variable de entorno en tiempo de
/// compilacion: cuando este binario de anfitrion existe, el kernel ya existe.
const KERNEL_ELF: &str = env!("CARGO_BIN_FILE_KERNEL_kernel");

/// Firmware UEFI OVMF tal como lo empaqueta Artix Linux (paquete `edk2-ovmf`).
/// Es la imagen combinada codigo+variables, apta para `-bios`.
const OVMF_POR_DEFECTO: &str = "/usr/share/edk2/x64/OVMF.4m.fd";

/// Nombre de la imagen de disco UEFI que renaser genera.
const NOMBRE_IMAGEN: &str = "renaser-uefi.img";

/// Ruta del disco de objetos del grafo persistente (Fase 6.1c). Relativa al
/// directorio de trabajo —la raiz del repo—, comun a `boot` y a QEMU.
const NOMBRE_DISCO: &str = "target/disk.img";

/// Tamaño del disco de objetos: 32 MiB. La imagen sembrada ocupa solo unos
/// pocos KiB; el resto queda a cero —espacio libre para que el grafo crezca—.
const TAM_DISCO: u64 = 32 * 1024 * 1024;

fn main() {
    if let Err(fallo) = orquestar() {
        // Un error de orquestacion se anuncia en rojo y aborta con codigo 1:
        // ninguna falla del anfitrion debe disfrazarse de exito.
        eprintln!("\x1b[1;31m[renaser/boot] fallo:\x1b[0m {fallo}");
        std::process::exit(1);
    }
}

/// Los modos de operacion del orquestador, elegidos por el primer argumento
/// de linea de comando. El DEFAULT (sin flags) es el bucle de desarrollo de
/// siempre — forjar y arrancar en QEMU; `--forjar`/`--install` son la CAPA R:
/// el grafo viaja DENTRO de la imagen como ramdisk para correr en metal sin
/// virtio-blk.
enum Modo {
    /// Forja la imagen SIN ramdisk y la arranca en QEMU con virtio-blk
    /// persistente. Los argumentos extra se reenvian a QEMU. (default)
    Qemu,
    /// Forja la imagen CON ramdisk embebido y se detiene — lista para flashear
    /// a un USB a mano. (`--forjar`)
    Forjar,
    /// Forja la imagen CON ramdisk y la escribe en un dispositivo de bloque
    /// real, tras triple confirmacion. (`--install /dev/sdX`)
    Instalar(String),
}

/// Lee el modo del primer flag reconocido. Todo lo que no sea `--forjar` /
/// `--install` se considera argumento de QEMU (`-display none`, `-d int`, …).
fn parsear_modo() -> Result<Modo, String> {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--forjar" => return Ok(Modo::Forjar),
            "--install" | "--instalar" => {
                let dispositivo = args.next().ok_or_else(|| {
                    "--install requiere un dispositivo de bloque, p.ej. --install /dev/sda"
                        .to_string()
                })?;
                return Ok(Modo::Instalar(dispositivo));
            }
            _ => {}
        }
    }
    Ok(Modo::Qemu)
}

/// Ejecuta, en orden, las operaciones de la Fase 1.5 segun el modo.
fn orquestar() -> Result<(), String> {
    let modo = parsear_modo()?;

    // --- 1. Localizar el artefacto del kernel. ---
    let kernel = Path::new(KERNEL_ELF);
    if !kernel.is_file() {
        return Err(format!(
            "no se encontro el ELF del kernel en {}\n  \
             (¿se interrumpio la compilacion de la dependencia de artefacto?)",
            kernel.display()
        ));
    }
    println!("[renaser/boot] kernel localizado :: {}", kernel.display());

    // --- 2. Sembrar el disco de objetos PRIMERO. Es a la vez la fuente del
    //        ramdisk (camino metal) y el drive virtio-blk (camino QEMU); por
    //        eso `set_ramdisk` mas abajo necesita que el archivo ya exista. ---
    preparar_disco_objetos()?;

    // --- 3. Forjar la imagen UEFI segun el modo y despacharla. ---
    let imagen = ruta_imagen(kernel);
    match modo {
        Modo::Qemu => {
            // Sin ramdisk: el kernel monta virtio-blk con persistencia real.
            forjar_imagen(kernel, &imagen, false)?;
            let ovmf = localizar_ovmf()?;
            lanzar_qemu(&imagen, &ovmf)
        }
        Modo::Forjar => {
            // Con ramdisk: el grafo viaja DENTRO de la imagen, sin depender de
            // virtio-blk (que no existe en metal).
            forjar_imagen(kernel, &imagen, true)?;
            println!(
                "\n[renaser/boot] imagen con ramdisk lista :: {0}\n  \
                 flashea a un USB con:\n    \
                 sudo dd if={0} of=/dev/sdX bs=4M conv=fsync status=progress\n  \
                 (o `cargo run -p boot -- --install /dev/sdX` para la version asistida)",
                imagen.display(),
            );
            Ok(())
        }
        Modo::Instalar(dispositivo) => {
            forjar_imagen(kernel, &imagen, true)?;
            instalar_en_dispositivo(&imagen, &dispositivo)
        }
    }
}

/// Forja la imagen de disco UEFI fusionando kernel + cargador. Si
/// `con_ramdisk`, embebe `target/disk.img` (el grafo sembrado) como ramdisk:
/// el cargador lo carga en RAM y se lo expone al kernel via `BootInfo.ramdisk_*`.
fn forjar_imagen(kernel: &Path, imagen: &Path, con_ramdisk: bool) -> Result<(), String> {
    println!(
        "[renaser/boot] forjando imagen UEFI{} :: {}",
        if con_ramdisk { " (con ramdisk)" } else { "" },
        imagen.display(),
    );
    let mut constructor = bootloader::UefiBoot::new(kernel);
    if con_ramdisk {
        constructor.set_ramdisk(Path::new(NOMBRE_DISCO));
    }
    constructor
        .create_disk_image(imagen)
        .map_err(|e| format!("la crate `bootloader` no pudo crear la imagen UEFI: {e:?}"))?;
    Ok(())
}

/// Escribe la imagen forjada en un dispositivo de bloque real. Por la gravedad
/// de la operacion —DESTRUYE todo dato previo— exige TRIPLE confirmacion
/// interactiva antes de invocar `sudo dd`.
fn instalar_en_dispositivo(imagen: &Path, dispositivo: &str) -> Result<(), String> {
    use std::io::{BufRead, Write as _};

    if !Path::new(dispositivo).exists() {
        return Err(format!("el dispositivo «{dispositivo}» no existe"));
    }

    // Mostrar el dispositivo para que el operador confirme con los ojos.
    println!("\n\x1b[1;31m[renaser/boot] INSTALACION EN HARDWARE REAL\x1b[0m");
    let _ = Command::new("lsblk")
        .args(["-o", "NAME,SIZE,MODEL,TRAN,MOUNTPOINT", dispositivo])
        .status();
    println!("\nEsto SOBRESCRIBE por completo «{dispositivo}». Todo dato previo se PIERDE.\n");

    let stdin = std::io::stdin();
    let preguntas = [
        format!("1/3 — ¿«{dispositivo}» es el USB correcto? escribe «si»: "),
        "2/3 — ¿entiendes que se BORRARA todo su contenido? escribe «borrar»: ".to_string(),
        format!("3/3 — reescribe el dispositivo completo para confirmar («{dispositivo}»): "),
    ];
    let esperadas = ["si", "borrar", dispositivo];
    for (pregunta, esperada) in preguntas.iter().zip(esperadas.iter()) {
        print!("{pregunta}");
        std::io::stdout().flush().ok();
        let mut linea = String::new();
        stdin
            .lock()
            .read_line(&mut linea)
            .map_err(|e| format!("no se pudo leer la confirmacion: {e}"))?;
        if linea.trim() != *esperada {
            return Err("instalacion abortada: la confirmacion no coincide".to_string());
        }
    }

    println!("\n[renaser/boot] escribiendo con `sudo dd` (puede pedir tu contraseña)…");
    let estado = Command::new("sudo")
        .arg("dd")
        .arg(format!("if={}", imagen.display()))
        .arg(format!("of={dispositivo}"))
        .arg("bs=4M")
        .arg("conv=fsync")
        .arg("status=progress")
        .status()
        .map_err(|e| format!("no se pudo invocar `sudo dd`: {e}"))?;
    if !estado.success() {
        return Err(format!("`dd` termino con estado anomalo: {estado}"));
    }
    let _ = Command::new("sync").status();
    println!("\n[renaser/boot] ✓ wawa instalado en «{dispositivo}». Reinicia y arranca desde el USB.");
    Ok(())
}

// =============================================================================
//  Fase 7b — la siembra del grafo: el userspace nace de la imagen de disco
// =============================================================================

/// Una app de genesis: su nombre legible, el `.wasm` que la encarna, la
/// ventana del framebuffer que habitara — `(x, y, ancho, alto)` en pixeles— y
/// su presupuesto de combustible por fotograma.
struct AppGenesis {
    nombre: &'static str,
    archivo: &'static str,
    region: (u32, u32, u32, u32),
    fuel: u32,
    /// Bitfield de permisos declarados en el manifiesto. El kernel los
    /// honrara en el momento de instanciar el modulo: las capacidades
    /// sensibles que no figuren aqui NO se registran en el `Linker` de
    /// wasmi, asi que el modulo no puede invocarlas — no por chequeo,
    /// sino porque no existen.
    permisos: u32,
}

/// Combustible por fotograma de una app comun: cubre con holgura un `tick`
/// de cientos de miles de operaciones, y una app en bucle infinito lo agota
/// en milisegundos.
const FUEL_COMUN: u32 = 2_000_000;

/// Combustible por fotograma del editor `bitacora`: re-resaltado tree-sitter
/// incremental, recompute de cursor y scroll caben holgadamente en 3x el
/// presupuesto comun. El primer caso real del modelo "fuel per-app".
const FUEL_EDITOR: u32 = 6_000_000;

/// El userspace de genesis — las catorce aplicaciones que pueblan un disco
/// recien forjado. La `bitacora` (Fase 17, editor que persiste), el `pregon`
/// (Fase 19, la primera voz hacia la red), la melodia visual `tonada` (Fase
/// 12), el compas visual `pulso` (Fase 11), un saludo (`hola`), la
/// `memoriosa` interactiva que recuerda entre sesiones (Fase 7c), tres
/// demos de los guardarrailes del kernel —`discola` (combustible),
/// `glotona` (memoria), `cronista` (la cronica de los arranques)—,
/// `tonalero` (Fase 22, testigo del bucle de Configuracion), `mudanza`
/// (Fase 25, el centro soberano de reancla del manifiesto: unica app con
/// PERMISO_RAIZ + sys_manifiesto_proponer), `asistente` (Fase 60, app
/// conversacional que habla con LLMs externos via el puente Linux) y
/// `rimay` (reflejo bare-metal del subdominio host de embeddings — demo
/// determinista de verbo + coseno sin daemon, sin red, sin descarga de
/// modelo).
const GENESIS: [AppGenesis; 16] = [
    AppGenesis { nombre: "bitacora", archivo: "bitacora.wasm", region: (100, 120, 480, 280), fuel: FUEL_EDITOR, permisos: 0 },
    AppGenesis { nombre: "pregon", archivo: "pregon.wasm", region: (100, 120, 480, 160), fuel: FUEL_COMUN, permisos: format::PERMISO_RED },
    AppGenesis { nombre: "tonada", archivo: "tonada.wasm", region: (100, 120, 360, 120), fuel: FUEL_COMUN, permisos: format::PERMISO_ALTAVOZ },
    AppGenesis { nombre: "pulso", archivo: "pulso.wasm", region: (100, 120, 360, 120), fuel: FUEL_COMUN, permisos: 0 },
    AppGenesis { nombre: "hola", archivo: "app.wasm", region: (100, 120, 480, 560), fuel: FUEL_COMUN, permisos: 0 },
    AppGenesis { nombre: "memoriosa", archivo: "memoriosa.wasm", region: (700, 120, 360, 80), fuel: FUEL_COMUN, permisos: 0 },
    AppGenesis { nombre: "discola", archivo: "discola.wasm", region: (60, 700, 360, 80), fuel: FUEL_COMUN, permisos: 0 },
    AppGenesis { nombre: "glotona", archivo: "glotona.wasm", region: (460, 700, 360, 80), fuel: FUEL_COMUN, permisos: 0 },
    AppGenesis { nombre: "cronista", archivo: "cronista.wasm", region: (860, 700, 360, 80), fuel: FUEL_COMUN, permisos: format::PERMISO_GRAFO_ESCRITURA | format::PERMISO_RAIZ },
    AppGenesis { nombre: "tonalero", archivo: "tonalero.wasm", region: (700, 220, 480, 300), fuel: FUEL_COMUN, permisos: format::PERMISO_CONFIG },
    AppGenesis { nombre: "mudanza", archivo: "mudanza.wasm", region: (60, 220, 480, 240), fuel: FUEL_COMUN, permisos: format::PERMISO_RAIZ },
    // Fase 33/34/35 :: `pluma` — la app bare-metal del notebook de Pluma.
    // Comparte tipos con `pluma-notebook-core` (no_std + alloc), render
    // distinto al de `pluma-notebook-llimphi` porque corre en framebuffer
    // 480x400 dentro de Wawa OS. PERMISO_GRAFO_ESCRITURA para encadenar
    // sys_object_put + sys_subsistema_registrar_ejecutable_v2 +
    // sys_subsistema_ejecutar_dinamico + sys_cuaderno_anexar_celda en
    // la cadena de F5. Sustituye al `ide` previo: el cuaderno hace todo lo
    // que el IDE hacia y ademas cascadea y persiste.
    AppGenesis { nombre: "pluma", archivo: "pluma.wasm", region: (160, 60, 480, 400), fuel: FUEL_EDITOR, permisos: format::PERMISO_GRAFO_ESCRITURA },
    // Fase 60 v5+v7 :: `asistente` — app conversacional que pregunta a un
    // LLM externo via el puente Linux (`asistente-puente --akasha`). El
    // protocolo cable usa EtherType 0x88B6 sobre `CANAL_ASISTENTE` (0x4153);
    // la app emite Consulta cuando el operador pulsa Enter y absorbe
    // Propuesta/Error. Para propuestas hash (Instalar/Cambiar) el operador
    // pulsa SPACE y la app dispara un RequestFirma; cuando llega la Firma
    // (host-side, ya sea desde `wawactl daemon-firma` o desde el propio
    // `asistente-puente --firma-clave`), pinta "FIRMADO POR SLOT N" e
    // (v7) invoca `sys_manifiesto_proponer` para cerrar el ciclo en una
    // sola transicion atomica del kernel — segunda app del genesis con
    // PERMISO_RAIZ, junto a `mudanza`. 480x240 es la geometria con la que
    // esta dibujada hoy; la region la coloca a la derecha del compositor
    // para no superponerse con mudanza, que esta abajo-izquierda.
    AppGenesis { nombre: "asistente", archivo: "asistente.wasm", region: (600, 220, 480, 240), fuel: FUEL_COMUN, permisos: format::PERMISO_RED | format::PERMISO_RAIZ },
    // `rimay` — reflejo bare-metal del subdominio host de embeddings.
    // Verbo determinista (FNV-1a + LCG, mismo algoritmo que
    // `rimay-verbo-mock`) + coseno sobre framebuffer 480x560. Sin
    // permisos especiales: no toca el grafo, no habla por red, no
    // necesita raíz — sólo framebuffer y teclado, las dos capacidades
    // que el kernel inyecta a toda app. La region se solapa con `hola`
    // (su mismo tamaño) — el operador elige cuál mirar.
    AppGenesis { nombre: "rimay", archivo: "rimay.wasm", region: (100, 120, 480, 560), fuel: FUEL_COMUN, permisos: 0 },
    // Fase C4 :: `testigo` — la app userspace que cierra el bucle del motor
    // `tinkuy` empotrado en el kernel. PERMISO_TINKUY le abre el grupo de
    // capacidades `sys_tinkuy_*`: sim_new + spawn × N + step_lj + observables
    // + snapshot_cid. No necesita nada mas — no toca el grafo, no habla por
    // red, no necesita raiz. 480x240 a la derecha del compositor para no
    // colisionar con `pluma` ni `asistente`.
    AppGenesis { nombre: "testigo", archivo: "testigo.wasm", region: (600, 520, 480, 240), fuel: FUEL_COMUN, permisos: format::PERMISO_TINKUY },
    // P6+ :: `ayni` — el chat soberano DENTRO de wawa, que HABLA POR LA RED. El
    // mismo `ayni-core` (no_std + alloc) que corre el chat en Linux, sin reescribir
    // su modelo, como app WASM. Tecleás un mensaje: se firma Ed25519, se persiste
    // como OBJETO del grafo de akasha —encadenado al anterior en una espina dorsal
    // que sobrevive a los reinicios, como la crónica de la `cronista`— y se DIFUNDE
    // en un frame Ethernet de EtherType propio (0x88B7), sin TCP/IP (akasha puro).
    // Otra instancia de wawa en el segmento absorbe el frame, verifica la firma e
    // integra el nodo: dos wawas convergen su conversación sin servidor. De ahí los
    // tres permisos: PERMISO_RED (sys_net_*), PERMISO_GRAFO_ESCRITURA + PERMISO_RAIZ
    // (sys_object_put + fijar_raiz). Primera app de genesis que funda su propio heap
    // (`alloc` lo exige el grafo). 480x400 a la derecha, como `rimay`.
    AppGenesis { nombre: "ayni", archivo: "ayni.wasm", region: (700, 120, 480, 400), fuel: FUEL_EDITOR, permisos: format::PERMISO_RED | format::PERMISO_GRAFO_ESCRITURA | format::PERMISO_RAIZ },
];

/// Techo de memoria lineal de cada app de genesis: 4 MiB. Un modulo que intente
/// crecer su memoria mas alla es desalojado por el kernel.
const TECHO_GENESIS: u32 = 4 * 1024 * 1024;

/// Garantiza la existencia del disco de objetos del grafo persistente. Si ya
/// existe, lo RESPETA — el grafo perdura entre arranques (la cuenta de la
/// cronista, el estado del userspace). Si no existe, lo forja Y LO SIEMBRA:
/// graba el grafo ya poblado con el bytecode de las apps y su Manifiesto de
/// Genesis. El kernel jamas vuelve a empotrar una sola app.
fn preparar_disco_objetos() -> Result<(), String> {
    let disco = Path::new(NOMBRE_DISCO);
    if disco.is_file() {
        println!(
            "[renaser/boot] disco de objetos presente :: {} — el grafo perdura",
            disco.display()
        );
        return Ok(());
    }
    if let Some(directorio) = disco.parent() {
        std::fs::create_dir_all(directorio)
            .map_err(|e| format!("no se pudo crear el directorio del disco de objetos: {e}"))?;
    }

    // Sembrar el grafo: el bytecode del userspace y el Manifiesto de Genesis.
    let (imagen, objetos) = sembrar_grafo()?;
    if imagen.len() as u64 > TAM_DISCO {
        return Err(format!(
            "el grafo sembrado ({} bytes) no cabe en el disco de objetos ({TAM_DISCO} bytes)",
            imagen.len()
        ));
    }

    // Escribir la imagen sembrada y extender el fichero a 32 MiB: el log queda
    // al principio; el resto, a cero —`set_len` lo deja disperso—.
    let mut fichero = std::fs::File::create(disco)
        .map_err(|e| format!("no se pudo crear el disco de objetos «{}»: {e}", disco.display()))?;
    fichero
        .write_all(&imagen)
        .map_err(|e| format!("no se pudo escribir el grafo sembrado: {e}"))?;
    fichero
        .set_len(TAM_DISCO)
        .map_err(|e| format!("no se pudo dimensionar el disco de objetos: {e}"))?;
    println!(
        "[renaser/boot] disco de objetos sembrado :: {} ({objetos} objetos, manifiesto anclado)",
        disco.display()
    );
    Ok(())
}

/// Lee el bytecode `.wasm` de una app de genesis desde `wawa-kernel/assets/`.
/// La ruta se ancla al directorio de ESTE crate —no al de trabajo—: el
/// constructor funciona se invoque desde donde se invoque.
///
/// FASE 64 :: corregido de `../kernel/assets` (stale del rename
/// `kernel`->`wawa-kernel`; ese path no existia y dependia de un symlink
/// local sin trackear) a `../wawa-kernel/assets`, el directorio real donde
/// `build-pluma.sh` consolida todos los `.wasm`. Sin esto, re-sembrar el
/// disco (necesario para tomar una app recien recompilada) fallaba.
fn leer_wasm(archivo: &str) -> Result<Vec<u8>, String> {
    let ruta = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../wawa-kernel/assets")
        .join(archivo);
    std::fs::read(&ruta)
        .map_err(|e| format!("no se pudo leer el bytecode «{}»: {e}", ruta.display()))
}

/// Lee, si existe, el objeto-concesion forjado fuera de banda para una app de
/// genesis: `wawa-kernel/assets/concesiones/<nombre>.cap.obj` (el payload
/// postcard de un `Objeto{datos:ConcesionCapacidad, hijos:[]}` que escribe
/// `agora-cli wawa concesion --salida`). Ausencia NO es error —es el caso comun
/// hasta que el operador complete la ceremonia §3.3—: devuelve `None`.
fn leer_concesion(nombre: &str) -> Result<Option<Vec<u8>>, String> {
    let ruta = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../wawa-kernel/assets/concesiones")
        .join(format!("{nombre}.cap.obj"));
    match std::fs::read(&ruta) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!(
            "no se pudo leer la concesion «{}»: {e}",
            ruta.display()
        )),
    }
}

/// Ancla la concesion de capacidad de una app de genesis, si el operador la
/// forjo. Verifica que la concesion firme EXACTAMENTE el objeto-bytecode que el
/// genesis acaba de grabar (`concesion.bytecode == bytecode`) —una concesion para
/// otro `.wasm` jamas se cuela—, la graba como objeto del grafo (dedup por
/// contenido) y la cuelga del manifiesto para que el MARK del GC la alcance.
/// Devuelve el hash a poner en `EntradaApp.concesion`, o `None` si no hay archivo.
fn sembrar_concesion(
    nombre: &str,
    bytecode: Hash,
    permisos: u32,
    log: &mut Vec<u8>,
    cursor: &mut u64,
    hijos: &mut Vec<Hash>,
    ancladas: &mut BTreeSet<Hash>,
) -> Result<Option<Hash>, String> {
    let payload = match leer_concesion(nombre)? {
        Some(p) => p,
        None => return Ok(None),
    };

    // El archivo es un `Objeto` del grafo cuyo payload es la `ConcesionCapacidad`.
    let objeto = Objeto::deserializar(&payload)
        .map_err(|e| format!("la concesion de «{nombre}» no es un Objeto valido: {e}"))?;
    let concesion = ConcesionCapacidad::deserializar(&objeto.datos)
        .map_err(|e| format!("la concesion de «{nombre}» es ilegible: {e}"))?;

    // GUARDA DURA: la firma cubre el bytecode. Si la app se recompilo sin
    // re-forjar la concesion, los hashes divergen y la concesion ya no aplica —
    // el kernel la rechazaria en silencio (efectivos = 0). Lo cazamos AQUI, en
    // voz alta, antes de sellar un disco que arrancaria una app castrada.
    if concesion.bytecode != bytecode {
        return Err(format!(
            "la concesion de «{nombre}» firma OTRO bytecode\n  \
             objeto-bytecode del genesis : {}\n  \
             cubierto por la concesion   : {}\n  \
             (¿se recompilo el .wasm sin re-forjar la concesion con `agora-cli wawa concesion`?)",
            hex_corto(&bytecode),
            hex_corto(&concesion.bytecode),
        ));
    }

    // Sanidad blanda: el kernel INTERSECTA (efectivos = declarados ∩ concedidos).
    // Si la concesion no cubre algun bit declarado, la app correra con MENOS de
    // lo que el manifiesto pide. No es un error —es el modelo— pero conviene avisar.
    if concesion.permisos & permisos != permisos {
        eprintln!(
            "[renaser/boot] aviso: «{nombre}» declara permisos {permisos:#x} pero su \
             concesion solo concede {:#x} — la interseccion recortara capacidades",
            concesion.permisos,
        );
    }

    // Anclar (una sola vez por contenido) y colgar del manifiesto.
    let hash = format::hash(&payload);
    if ancladas.insert(hash) {
        anexar_objeto(log, cursor, &payload)?;
        hijos.push(hash);
    }
    Ok(Some(hash))
}

/// Los primeros 8 bytes de un hash en hex — suficiente para distinguir objetos
/// en un mensaje de diagnostico sin volcar los 32 bytes completos.
fn hex_corto(hash: &Hash) -> String {
    hash.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Anexa un objeto al log: compone su registro `[longitud][payload][relleno]`,
/// lo añade a la imagen y avanza el cursor. Devuelve el hash del objeto — su
/// identidad en el grafo direccionado por contenido.
fn anexar_objeto(log: &mut Vec<u8>, cursor: &mut u64, payload: &[u8]) -> Result<Hash, String> {
    if payload.is_empty() || payload.len() > MAX_OBJETO {
        return Err(format!(
            "un objeto del grafo tiene un tamaño invalido: {} bytes",
            payload.len()
        ));
    }
    let hash = format::hash(payload);
    log.extend_from_slice(&format::componer_registro(payload));
    *cursor += format::sectores_registro(payload.len());
    Ok(hash)
}

/// Siembra el grafo de objetos de un disco virgen: graba el bytecode de cada
/// app de genesis como un objeto del grafo, compone el Manifiesto de Genesis
/// —con sus regiones y cuotas—, lo graba con las aristas hacia los objetos de
/// bytecode, y forja el superbloque que lo ancla. Devuelve la imagen del disco
/// (superbloque en el sector 0 + el log de registros) y el numero de objetos
/// sembrados. Habla, byte a byte, el format que el kernel leera al montar.
fn sembrar_grafo() -> Result<(Vec<u8>, usize), String> {
    // El log de registros: del sector 1 en adelante. El sector 0 es el
    // superbloque, que aun no podemos escribir —no conocemos el cursor final—.
    let mut log: Vec<u8> = Vec::new();
    let mut cursor: u64 = 1;

    // --- 1. Los objetos de bytecode, DEDUPLICADOS por archivo. Dos apps que
    //        comparten el mismo `.wasm` comparten un unico objeto del grafo. ---
    let mut hash_de: BTreeMap<&str, Hash> = BTreeMap::new();
    let mut hijos_manifiesto: Vec<Hash> = Vec::new();
    let mut apps: Vec<EntradaApp> = Vec::new();
    // Concesiones de capacidad ya ancladas (dedup por contenido): dos apps con
    // el mismo bytecode + permisos + firmante comparten un unico objeto-concesion.
    let mut concesiones: BTreeSet<Hash> = BTreeSet::new();

    for app in &GENESIS {
        let bytecode = match hash_de.get(app.archivo) {
            // Ya grabado: el grafo no guarda dos veces el mismo contenido.
            Some(&hash) => hash,
            None => {
                let datos = leer_wasm(app.archivo)?;
                let objeto = Objeto { datos, hijos: Vec::new() };
                let payload = objeto.serializar().map_err(|e| e.to_string())?;
                let hash = anexar_objeto(&mut log, &mut cursor, &payload)?;
                hash_de.insert(app.archivo, hash);
                hijos_manifiesto.push(hash);
                hash
            }
        };
        // SEAM de la ceremonia §14.1.3 (SDD-capacidades §3.3). `boot` NO tiene
        // clave privada —la seed del `AGORA_AUTH_RING` vive offline con el
        // operador—, asi que NO firma concesiones aqui. Pero SI ancla las que el
        // operador forjo fuera de banda (`agora-cli wawa concesion`) y dejo en
        // `wawa-kernel/assets/concesiones/<nombre>.cap.obj`. Solo se buscan para
        // apps con permisos gateados; las de `permisos == 0` no necesitan
        // ceremonia. Si no hay archivo, `None` ⇒ el rollout escalonado (SDD §3.6)
        // honra `permisos` tal cual: cero cambio de comportamiento sin provisionar.
        let concesion = if app.permisos != 0 {
            sembrar_concesion(
                app.nombre,
                bytecode,
                app.permisos,
                &mut log,
                &mut cursor,
                &mut hijos_manifiesto,
                &mut concesiones,
            )?
        } else {
            None
        };

        let (x, y, ancho, alto) = app.region;
        apps.push(EntradaApp {
            nombre: app.nombre.to_string(),
            bytecode,
            region_x: x,
            region_y: y,
            region_ancho: ancho,
            region_alto: alto,
            techo_memoria: TECHO_GENESIS,
            fuel_fotograma: app.fuel,
            estado: None,
            permisos: app.permisos,
            concesion,
        });
    }

    // --- 2. El objeto del Manifiesto de Genesis. Sus `hijos` son los objetos
    //        de bytecode: el grafo lo lee como el nodo padre del userspace. ---
    // Sin configuracion enlazada: el kernel inyectara `Configuracion::por_defecto`
    // en cada `ContextoCapacidades`. El cambio de idioma/tema engendrara un
    // nodo nuevo en caliente y reanclara el manifiesto sin pasar por aqui.
    let manifiesto = Manifiesto { version: VERSION_MANIFIESTO, apps, configuracion: None };
    let man_datos = manifiesto.serializar().map_err(|e| e.to_string())?;
    let man_objeto = Objeto { datos: man_datos, hijos: hijos_manifiesto };
    let man_payload = man_objeto.serializar().map_err(|e| e.to_string())?;
    let hash_manifiesto = anexar_objeto(&mut log, &mut cursor, &man_payload)?;

    // El grafo sembrado: un objeto por cada `.wasm` unico, uno por cada
    // concesion de capacidad anclada, mas el manifiesto.
    let objetos = hash_de.len() + concesiones.len() + 1;

    // --- 3. El superbloque: el ancla del grafo, en el sector 0. `raiz` queda
    //        vacia —el userspace la fija; `manifiesto` apunta a la genesis. ---
    let superbloque = SuperBloque {
        magia: MAGIA,
        version: VERSION_SUPERBLOQUE,
        // Un disco recien sembrado arranca el log justo despues del
        // superbloque. El compactador semantico lo desplazara mas tarde.
        log_inicio: 1,
        cursor,
        raiz: None,
        manifiesto: Some(hash_manifiesto),
    };
    let sb_bytes = superbloque.serializar().map_err(|e| e.to_string())?;
    if sb_bytes.len() > TAM_SECTOR {
        return Err("el superbloque sembrado no cabe en un sector".to_string());
    }

    // La imagen: el superbloque en el sector 0 (relleno a cero) y, tras el, el
    // log de registros que acabamos de componer.
    let mut imagen = vec![0u8; TAM_SECTOR];
    imagen[..sb_bytes.len()].copy_from_slice(&sb_bytes);
    imagen.extend_from_slice(&log);

    Ok((imagen, objetos))
}

/// Calcula la ruta de la imagen: junto al propio ELF del kernel, es decir,
/// dentro de `target/`. Una ubicacion predecible y siempre escribible.
fn ruta_imagen(kernel: &Path) -> PathBuf {
    kernel
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(NOMBRE_IMAGEN)
}

/// Resuelve la ruta del firmware OVMF. Permite sobreescribirla con la variable
/// de entorno `RENASER_OVMF` para entornos cuyo `edk2-ovmf` viva en otra ruta.
fn localizar_ovmf() -> Result<String, String> {
    let ruta = std::env::var("RENASER_OVMF").unwrap_or_else(|_| OVMF_POR_DEFECTO.to_string());
    if Path::new(&ruta).is_file() {
        Ok(ruta)
    } else {
        Err(format!(
            "firmware UEFI OVMF no encontrado en «{ruta}»\n  \
             instala el paquete `edk2-ovmf`, o exporta RENASER_OVMF=<ruta a OVMF.fd>"
        ))
    }
}

/// Invoca QEMU como subproceso. Los argumentos se ciñen a las primitivas
/// minimas necesarias para que el Framebuffer GOP cobre vida:
///
///   * `-bios`            firmware UEFI OVMF.
///   * `-drive raw`       la imagen de disco UEFI, sin capa de traduccion.
///   * `-vga none` +      FASE 60 :: `virtio-vga` es un virtio-gpu CON
///     `virtio-vga`       compatibilidad VGA: el firmware OVMF le bindea su
///                        driver de video estandar y expone el framebuffer GOP
///                        que el arranque necesita —igual que `-vga std`—,
///                        mientras que sobre PCI es un virtio-gpu de verdad que
///                        el kernel reclama (`drivers::gpu`) para tomar posesion
///                        del scanout. `-vga none` evita un segundo VGA en
///                        conflicto. Si el kernel no logra montarlo, recae al
///                        GOP que OVMF dejo sobre este mismo dispositivo.
///   * `-serial stdio`    telemetria serial del procesador hacia esta consola.
///   * `--no-reboot`      un fallo triple detiene la maquina en vez de reiniciar
///                        en bucle: asi la baliza de panico permanece visible.
///   * `virtio-blk-pci`   el disco de objetos, sobre el bus PCI (q35 es x86_64;
///                        `virtio-blk-device`, su gemelo MMIO, es cosa de ARM).
fn lanzar_qemu(imagen: &Path, ovmf: &str) -> Result<(), String> {
    println!("[renaser/boot] arrancando QEMU :: la superficie indigo nace ahora\n");

    let mut qemu = Command::new("qemu-system-x86_64");
    // `accel=kvm:tcg` intenta KVM y, si no esta disponible, recae en TCG puro.
    qemu.arg("-machine").arg("q35,accel=kvm:tcg")
        .arg("-m").arg("256M")
        .arg("-bios").arg(ovmf)
        .arg("-drive").arg(format!("format=raw,file={}", imagen.display()))
        // FASE 60 :: virtio-gpu con compatibilidad VGA. OVMF expone su GOP para
        // el arranque; el kernel reclama el mismo dispositivo y toma el scanout.
        .arg("-vga").arg("none")
        .arg("-device").arg("virtio-vga")
        .arg("-serial").arg("stdio")
        .arg("--no-reboot")
        // El disco de objetos, como dispositivo virtio-blk sobre el bus PCI.
        .arg("-drive").arg(format!("format=raw,file={NOMBRE_DISCO},if=none,id=drv0"))
        .arg("-device").arg("virtio-blk-pci,drive=drv0")
        // FASE 61 :: tableta virtio-input — puntero ABSOLUTO. QEMU enruta el
        // cursor del host a este dispositivo (coordenadas absolutas), de modo
        // que el puntero del huesped lo sigue 1:1, sin captura ni deriva.
        .arg("-device").arg("virtio-tablet-pci")
        // FASE 62 :: virtio-sound — PCM real por DMA. El backend de audio es
        // `none` por DEFECTO: el dispositivo funciona (el kernel ejercita todo
        // el camino PCM) pero el host NO emite sonido — asi NO arriesgamos
        // romper el arranque en una maquina sin PulseAudio/PipeWire. Para OIRLO,
        // cambia `none` por `pa` (PulseAudio/pipewire-pulse), `pipewire` o `sdl`.
        .arg("-audiodev").arg("none,id=snd0")
        .arg("-device").arg("virtio-sound-pci,audiodev=snd0");

    // FASE 67 :: la tarjeta de red. Dos backends segun para que arrancamos:
    //
    //   * DEFECTO — `user mode networking` de QEMU: un NAT virtual hacia el host
    //     (gateway 10.0.2.2, DHCP/DNS 10.0.2.3, invitado 10.0.2.15). El kernel
    //     emite un ARP al gateway como prueba de vida. PERO el NAT de QEMU solo
    //     reenvia IP: NO transporta el EtherType propio de Akasha (0x88B5). Por
    //     eso el camino vivo de `mudanza` (host `agora-cli wawa anunciar` -> wire
    //     -> guest) NO funciona sobre `user`.
    //
    //   * `RENASER_TAP=<iface>` — bridgea la NIC del guest a un dispositivo TAP
    //     del host. Un TAP transporta CUALQUIER EtherType en capa-2, asi que el
    //     host puede abrir un raw socket sobre el mismo `tap0` y difundir AoE que
    //     el guest recibe 1:1 — el unico transporte que cierra el bucle de
    //     re-ancla en red contra QEMU. Crealo antes con `scripts/aoe-tap-setup.sh`
    //     (o `ip tuntap add tap0 mode tap user $USER && ip link set tap0 up`) y
    //     corre `sudo -E agora-cli wawa anunciar --iface tap0 --dir <release>`.
    //     `script=no,downscript=no`: QEMU no invoca el helper de red de root —el
    //     tap ya existe y esta arriba, solo lo adopta.
    match std::env::var("RENASER_TAP") {
        Ok(tap) if !tap.is_empty() => {
            println!("[renaser/boot] red :: TAP «{tap}» (AoE host<->guest habilitado)");
            qemu.arg("-netdev")
                .arg(format!("tap,id=net0,ifname={tap},script=no,downscript=no"))
                .arg("-device")
                .arg("virtio-net-pci,netdev=net0");
        }
        _ => {
            qemu.arg("-netdev")
                .arg("user,id=net0")
                .arg("-device")
                .arg("virtio-net-pci,netdev=net0");
        }
    }

    // Cualquier argumento extra tras `--` se reenvia a QEMU intacto.
    // Ejemplo: `cargo run -p boot -- -display none -d int`.
    qemu.args(std::env::args().skip(1));

    match qemu.status() {
        Ok(estado) if estado.success() => {
            println!("\n[renaser/boot] QEMU finalizo limpiamente.");
            Ok(())
        }
        Ok(estado) => Err(format!("QEMU termino con estado anomalo: {estado}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(
            "`qemu-system-x86_64` no esta en el PATH; instala el paquete `qemu-full`".to_string(),
        ),
        Err(e) => Err(format!("no se pudo ejecutar QEMU: {e}")),
    }
}
