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

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

// El format del grafo de objetos en disco — el MISMO nucleo `no_std` que
// enlaza el kernel. Gracias a el, lo que `boot` siembra y lo que el kernel lee
// es, byte a byte, el mismo idioma.
use format::{
    EntradaApp, Hash, Manifiesto, Objeto, SuperBloque, MAGIA, MAX_OBJETO, TAM_SECTOR,
    VERSION_MANIFIESTO, VERSION_SUPERBLOQUE,
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

/// Ejecuta, en orden, las operaciones de la Fase 1.5.
fn orquestar() -> Result<(), String> {
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

    // --- 2. Fusionar kernel + cargador UEFI en una imagen de disco. ---
    let imagen = ruta_imagen(kernel);
    println!("[renaser/boot] forjando imagen UEFI :: {}", imagen.display());
    bootloader::UefiBoot::new(kernel)
        .create_disk_image(&imagen)
        .map_err(|e| format!("la crate `bootloader` no pudo crear la imagen UEFI: {e:?}"))?;

    // --- 3. Garantizar —y, si es virgen, SEMBRAR— el disco de objetos. ---
    preparar_disco_objetos()?;

    // --- 4. Lanzar QEMU sobre esa imagen. ---
    let ovmf = localizar_ovmf()?;
    lanzar_qemu(&imagen, &ovmf)
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

/// El userspace de genesis — las once aplicaciones que pueblan un disco recien
/// forjado. La `bitacora` (Fase 17, editor que persiste), el `pregon` (Fase 19,
/// la primera voz hacia la red), la melodia visual `tonada` (Fase 12), el
/// compas visual `pulso` (Fase 11), un saludo (`hola`), la `memoriosa`
/// interactiva que recuerda entre sesiones (Fase 7c), tres demos de los
/// guardarrailes del kernel —`discola` (combustible), `glotona` (memoria),
/// `cronista` (la cronica de los arranques)—, `tonalero` (Fase 22, testigo
/// del bucle de Configuracion) y `mudanza` (Fase 25): el centro soberano
/// de reancla del manifiesto, unica app con PERMISO_RAIZ + sys_manifiesto_proponer.
const GENESIS: [AppGenesis; 12] = [
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

/// Lee el bytecode `.wasm` de una app de genesis desde `kernel/assets/`. La
/// ruta se ancla al directorio de ESTE crate —no al de trabajo—: el
/// constructor funciona se invoque desde donde se invoque.
fn leer_wasm(archivo: &str) -> Result<Vec<u8>, String> {
    let ruta = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../kernel/assets")
        .join(archivo);
    std::fs::read(&ruta)
        .map_err(|e| format!("no se pudo leer el bytecode «{}»: {e}", ruta.display()))
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

    // El grafo sembrado: un objeto por cada `.wasm` unico, mas el manifiesto.
    let objetos = hash_de.len() + 1;

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
///   * `-vga std`         VGA estandar => framebuffer lineal que el GOP expone.
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
        .arg("-vga").arg("std")
        .arg("-serial").arg("stdio")
        .arg("--no-reboot")
        // El disco de objetos, como dispositivo virtio-blk sobre el bus PCI.
        .arg("-drive").arg(format!("format=raw,file={NOMBRE_DISCO},if=none,id=drv0"))
        .arg("-device").arg("virtio-blk-pci,drive=drv0")
        // FASE 18 :: la tarjeta de red — `user mode networking` de QEMU, un
        // NAT virtual hacia el host. Sin opciones extra: gateway en 10.0.2.2,
        // DHCP/DNS en 10.0.2.3, el invitado en 10.0.2.15. El kernel envia un
        // ARP request al gateway en cuanto arranca como prueba de vida.
        .arg("-netdev").arg("user,id=net0")
        .arg("-device").arg("virtio-net-pci,netdev=net0");

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
