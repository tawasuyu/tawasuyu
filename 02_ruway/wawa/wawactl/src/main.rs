//! `wawactl` — CLI sobre `wawa-config`.
//!
//! Subcomandos:
//!
//!   wawactl path [--system]            imprime el path del archivo de la capa
//!   wawactl show [--json] [--layer …]  dump completo de una capa o la efectiva
//!   wawactl get <key>                  imprime el valor efectivo de una key
//!   wawactl set <key> <value> [--system]   escribe (defaults a capa usuario)
//!   wawactl reset [--system]           restablece defaults y persiste
//!   wawactl watch                      sigue ambas capas y muestra cada cambio
//!   wawactl module <id> on|off|toggle [--system]   conmuta un módulo
//!   wawactl claves forjar --slot N --salida PATH    forja un par Ed25519
//!   wawactl claves derivar-pubkey --clave-privada PATH  re-deriva pubkey
//!
//! Las keys aceptadas en get/set son los nombres de campos del struct:
//! `theme_variant`, `accent`, `lang`, `timefmt_24h`. Para los módulos
//! existe el subcomando dedicado `module`.
//!
//! Capas:
//!   - **user** (default) → `$XDG_CONFIG_HOME/wawa/config.json`.
//!   - **system** (`--system`) → `/etc/wawa/config.json` (Linux,
//!     requiere root para escribir).
//!   - **effective** → unión mergeada usuario sobre sistema, para
//!     leer lo que realmente ven las apps.

use std::process::ExitCode;
use std::sync::mpsc;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use wawa_config::{ConfigWatcher, Layer, WawaConfig};

#[derive(Parser)]
#[command(name = "wawactl", about = "Cliente CLI del bus de configuración wawa-config")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Imprime el path absoluto del archivo de configuración. Por
    /// defecto, el de la capa de usuario; con `--system` el de
    /// `/etc/wawa/config.json`.
    Path {
        #[arg(long)]
        system: bool,
    },
    /// Muestra la configuración. Por defecto la efectiva (sistema +
    /// usuario mergeada). `--layer system` o `--layer user` para
    /// inspeccionar una capa concreta sin mergear.
    Show {
        #[arg(long)]
        json: bool,
        #[arg(long, value_enum, default_value_t = ShowLayer::Effective)]
        layer: ShowLayer,
    },
    /// Devuelve el valor efectivo de una key (post-merge sistema +
    /// usuario).
    Get {
        /// `theme_variant`, `accent`, `lang`, `timefmt_24h`.
        key: String,
    },
    /// Cambia una key y persiste atómicamente en la capa de usuario.
    /// Con `--system` escribe en `/etc/wawa/config.json` (requiere
    /// root).
    Set {
        /// `theme_variant`, `accent`, `lang`, `timefmt_24h`.
        key: String,
        /// Para `timefmt_24h` acepta `true`/`false`/`24`/`12`.
        value: String,
        #[arg(long)]
        system: bool,
    },
    /// Restablece la capa indicada a defaults. Por defecto la de
    /// usuario; con `--system` la de sistema (requiere root).
    Reset {
        #[arg(long)]
        system: bool,
    },
    /// Subscribe al bus (ambas capas) e imprime cada cambio. Útil
    /// para debugging: dejar corriendo en una terminal mientras se
    /// prueba el panel.
    Watch,
    /// Conmuta un módulo del SO. Por defecto en la capa de usuario;
    /// con `--system` en la de sistema (requiere root).
    Module {
        /// id del módulo (`mirada`, `shuma`, …)
        id: String,
        /// `on`, `off`, o `toggle`.
        op: String,
        #[arg(long)]
        system: bool,
    },
    /// Fase 38 :: firma un cuaderno de Pluma para que el kernel de Wawa
    /// lo ancle como soberano. Lee el hash en hexadecimal (64 chars), la
    /// clave privada Ed25519 desde un archivo (32 bytes en formato
    /// raw — el formato esperado por `ed25519-compact`), genera los 64
    /// bytes de la firma y los emite a stdout. El operador los redirige
    /// al canal serial de QEMU (por ejemplo `> /dev/ttyS0` cuando el
    /// dispositivo COM1 esta mapeado a un PTY).
    ///
    /// Uso tipico:
    ///   wawactl firmar-cuaderno \
    ///       --hash 0123...abcd \
    ///       --clave-privada ~/.config/wawa/operador.sk \
    ///       > /dev/ttyS0
    ///
    /// El comando es minimal por diseno: NO abre la PTY de QEMU el solo,
    /// NO escucha COM1 esperando solicitudes. Es un primitive que el
    /// arnes de pruebas / un wrapper futuro de mas alto nivel puede
    /// componer. La criptografia es real (ed25519-compact); cuando
    /// `wawactl` arme su capa de "demonio escucha", la integracion sera
    /// directa.
    FirmarCuaderno {
        /// Hash del cuaderno a firmar, 64 caracteres hexadecimales.
        #[arg(long)]
        hash: String,
        /// Path al archivo con la clave privada Ed25519 cruda (32 bytes).
        #[arg(long = "clave-privada")]
        clave_privada: String,
    },
    /// Fase 39 :: demonio bidireccional. Escucha el PTY que QEMU expone
    /// como COM1 de Wawa, parsea solicitudes `wawa::sign_request::<HEX>`,
    /// pide confirmacion al operador humano (`y/N`, timeout 30 s) y
    /// reinyecta la firma Ed25519 de 64 bytes raw de vuelta por el mismo
    /// canal. Cada firma autorizada deja huella en
    /// `wawactl_audit.log` para analisis forense.
    ///
    /// Uso tipico (con QEMU mapeando COM1 a un PTY local):
    ///   qemu-system-x86_64 -serial pty ... # imprime "char device redirected to /dev/pts/N"
    ///   wawactl daemon-firma \
    ///       --pty /dev/pts/N \
    ///       --clave-privada ~/.config/wawa/operador.sk
    ///
    /// El demonio NO firma ciegamente — la soberania del operador es
    /// inviolable. Una app enjaulada que inunde el COM1 con
    /// sign_requests solo lograra que el humano vea el spam y rechace.
    /// FASE 48 :: ceremonia de fianza de claves soberanas (Boot Trust
    /// Ceremony). Subcomandos:
    ///
    ///   wawactl claves forjar --slot <0|1|2> --salida <PATH>
    ///     Forja un par Ed25519 fresco con entropia del SO, persiste la
    ///     seed (32 B) en `--salida` con permisos 0600 e imprime la
    ///     clave publica derivada como un array literal de Rust listo
    ///     para pegar en `kernel/src/claves.rs::AGORA_AUTH_RING`.
    ///
    ///   wawactl claves derivar-pubkey --clave-privada <PATH>
    ///     Re-imprime la clave publica de una seed persistida — utilidad
    ///     forense del operador antes de re-forjar la imagen del kernel.
    Claves {
        #[command(subcommand)]
        op: ClavesCmd,
    },
    /// Fase 63 :: dispara una compactacion del grafo del kernel de Wawa de
    /// forma remota, sobre el MISMO virtio-console que `daemon-firma`. Emite
    /// `wawactl::gc_request::` por el canal y espera el veredicto
    /// `wawactl::gc_reply::vivos=N muertos=M sectores=A->B` (timeout 30 s).
    /// Es la cara host-side de la syscall `sys_grafo_compactar` (Fase 53) y
    /// de la palanca `Alt+G` (Fase 57) — el operador fuerza el GC desde el
    /// anfitrion sin tocar el teclado del huesped.
    ///
    /// Uso tipico (QEMU con virtio-console mapeado a un char device/socket):
    ///   wawactl gc --char-device /dev/pts/N
    Gc {
        /// Path al char device / socket del virtio-console (el mismo backend
        /// que `daemon-firma --char-device`). Alias: `--virtio-port`.
        #[arg(long = "char-device")]
        char_device: Option<String>,
        /// Alias de `--char-device`.
        #[arg(long = "virtio-port")]
        virtio_port: Option<String>,
        /// Segundos a esperar el veredicto antes de rendirse. Default 30.
        #[arg(long, default_value_t = 30u64)]
        timeout: u64,
    },
    DaemonFirma {
        /// LEGACY (Fase 38/39) :: path al PTY/dispositivo serial que QEMU
        /// expone como COM1. Activa el parser ASCII
        /// `wawa::sign_request::<HEX>\n`. Mutuamente excluyente con
        /// `--char-device` / `--virtio-port`.
        #[arg(long, conflicts_with_all = ["char_device", "virtio_port"])]
        pty: Option<String>,
        /// FASE 49 :: path al char device / socket que QEMU expone como
        /// backend del virtio-console (`-chardev socket,path=... ; -device
        /// virtconsole,chardev=...`). Activa el parser BINARIO
        /// `wawactl::sign_pci::<32 raw bytes>`. Alias: `--virtio-port`.
        #[arg(long = "char-device", conflicts_with = "pty")]
        char_device: Option<String>,
        /// Alias de `--char-device` (mas conciso en docs/qemu invocations).
        #[arg(long = "virtio-port", conflicts_with = "pty")]
        virtio_port: Option<String>,
        /// Path al archivo con la clave privada Ed25519 (32 B seed o 64 B SK).
        #[arg(long = "clave-privada")]
        clave_privada: String,
        /// Path opcional del log de auditoria. Default:
        /// `./wawactl_audit.log`.
        #[arg(long = "log", default_value = "wawactl_audit.log")]
        log: String,
        /// FASE 41 :: ventana opcional de PRE-AUTORIZACION temporal. Si se
        /// indica, durante esa ventana las solicitudes se firman
        /// automaticamente (sin prompt) y se registran en el audit log
        /// con el marcador `FIRMA_AUTO_EMITIDA`. Al expirar, el demonio
        /// vuelve al modo restrictivo por defecto (confirmacion interactiva
        /// con timeout). Sintaxis: `30s`, `15m`, `1h` (s/m/h).
        #[arg(long = "auto-firmar-durante")]
        auto_firmar_durante: Option<String>,
        /// FASE 42 :: slot del anillo multi-autor (`AGORA_AUTH_RING`) con
        /// el que esta clave firma. 0 = primaria (default), 1 = secundaria,
        /// 2 = recuperacion. El demonio antepone este byte al sello
        /// Ed25519 de 64 B para que `apps/pluma` autodetecte que clave
        /// publica embeber en el sobre `CuadernoFirmado`.
        #[arg(long, default_value_t = 0u8)]
        slot: u8,
    },
}

/// FASE 48 :: variantes del subcomando `wawactl claves`. Cada una opera
/// estrictamente offline; la criptografia ocurre del lado del operador y
/// el kernel jamas ve la clave privada.
#[derive(Subcommand)]
enum ClavesCmd {
    /// Forja un par Ed25519 fresco con entropia del SO. Persiste la seed
    /// (32 B raw) en `--salida` con permisos 0600 e imprime la clave
    /// publica como array literal de Rust, listo para inyectar en el
    /// slot indicado del anillo `AGORA_AUTH_RING` del kernel.
    Forjar {
        /// Slot del anillo multi-autor (0=primaria, 1=secundaria,
        /// 2=recuperacion). Anota el destino del literal en la cabecera
        /// del bloque impreso a stdout — no muta el archivo del kernel.
        #[arg(long)]
        slot: u8,
        /// Path donde persistir la clave privada (seed de 32 bytes raw).
        /// Si el archivo ya existe, se aborta sin sobrescribir — la
        /// soberania del operador es inviolable.
        #[arg(long)]
        salida: String,
    },
    /// Re-deriva la clave publica de una seed persistida. Imprime el
    /// literal Rust por stdout. Operacion forense: util para auditar
    /// que la clave publica grabada en el kernel sigue casando con la
    /// seed que el operador guarda en su HSM/USB.
    DerivarPubkey {
        /// Path al archivo con la clave privada Ed25519 (32 B seed
        /// raw o 64 B SecretKey completa — ambos formatos aceptados).
        #[arg(long = "clave-privada")]
        clave_privada: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum ShowLayer {
    /// Sólo la capa de sistema (`/etc/wawa/config.json`), sin merge.
    System,
    /// Sólo la capa de usuario, sin merge.
    User,
    /// Unión mergeada usuario sobre sistema — lo que ven las apps.
    Effective,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("wawactl: {msg}");
            ExitCode::from(1)
        }
    }
}

fn run(cmd: Cmd) -> Result<(), String> {
    match cmd {
        Cmd::Path { system } => cmd_path(layer_of(system)),
        Cmd::Show { json, layer } => cmd_show(json, layer),
        Cmd::Get { key } => cmd_get(&key),
        Cmd::Set { key, value, system } => cmd_set(&key, &value, layer_of(system)),
        Cmd::Reset { system } => cmd_reset(layer_of(system)),
        Cmd::Watch => cmd_watch(),
        Cmd::Module { id, op, system } => cmd_module(&id, &op, layer_of(system)),
        Cmd::FirmarCuaderno { hash, clave_privada } => cmd_firmar_cuaderno(&hash, &clave_privada),
        Cmd::Claves { op } => match op {
            ClavesCmd::Forjar { slot, salida } => cmd_claves_forjar(slot, &salida),
            ClavesCmd::DerivarPubkey { clave_privada } => cmd_claves_derivar_pubkey(&clave_privada),
        },
        Cmd::Gc {
            char_device,
            virtio_port,
            timeout,
        } => {
            let path = char_device.or(virtio_port).ok_or_else(|| {
                "gc requiere --char-device <PATH> (o --virtio-port), el backend del \
                 virtio-console expuesto por QEMU"
                    .to_string()
            })?;
            cmd_gc(&path, timeout)
        }
        Cmd::DaemonFirma {
            pty,
            char_device,
            virtio_port,
            clave_privada,
            log,
            auto_firmar_durante,
            slot,
        } => {
            // FASE 49 :: el transporte se infiere del flag. `--char-device`
            // y `--virtio-port` son alias del bus virtio; `--pty` es el
            // legacy UART. Exactamente UNO debe estar presente — clap ya
            // hace cumplir conflicts; aqui completamos con la regla "al
            // menos uno". Sin flag, abortamos con ayuda explicita.
            let virtio = char_device.or(virtio_port);
            let (transporte_path, modo) = match (pty, virtio) {
                (Some(p), None) => (p, ModoTransporte::PtyAscii),
                (None, Some(p)) => (p, ModoTransporte::VirtioBinario),
                (None, None) => {
                    return Err(
                        "daemon-firma requiere --pty <PATH> (legacy) o --char-device <PATH> \
                         (virtio-console, Fase 49)"
                            .to_string(),
                    );
                }
                (Some(_), Some(_)) => unreachable!("clap rechaza el conflicto antes"),
            };
            cmd_daemon_firma(
                &transporte_path,
                modo,
                &clave_privada,
                &log,
                auto_firmar_durante.as_deref(),
                slot,
            )
        }
    }
}

/// FASE 38 :: subcomando `firmar-cuaderno`. Lee el hash hex, carga la clave
/// privada Ed25519 desde un archivo, firma los 32 bytes del hash y vuelca
/// los 64 bytes crudos de la firma a stdout. Operacion offline: no toca el
/// kernel ni la red — el redireccionado al PTY de QEMU vive en el shell
/// del operador. ZERO heuristica: si el hash no es 64 hex o la clave no es
/// 32 bytes, abortamos sin firmar.
fn cmd_firmar_cuaderno(hash_hex: &str, clave_path: &str) -> Result<(), String> {
    use std::io::Write;

    if hash_hex.len() != 64 {
        return Err(format!(
            "el hash debe traer 64 caracteres hexadecimales; recibi {}",
            hash_hex.len()
        ));
    }
    let mut hash = [0u8; 32];
    for i in 0..32 {
        let byte_str = &hash_hex[i * 2..i * 2 + 2];
        hash[i] = u8::from_str_radix(byte_str, 16)
            .map_err(|_| format!("hash con caracteres no-hex cerca del byte {i}"))?;
    }

    let clave_bytes = std::fs::read(clave_path)
        .map_err(|e| format!("no pude leer la clave privada en {clave_path}: {e}"))?;
    // ed25519-compact espera la SecretKey en formato "expanded" (64 bytes:
    // 32 de la seed + 32 de la pubkey derivada). Aceptamos ambos formatos:
    // si el archivo trae 32 bytes los tratamos como seed; si trae 64, como
    // SecretKey completa.
    let firma = if clave_bytes.len() == 32 {
        let mut seed_arr = [0u8; 32];
        seed_arr.copy_from_slice(&clave_bytes);
        let seed = ed25519_compact::Seed::new(seed_arr);
        let kp = ed25519_compact::KeyPair::from_seed(seed);
        kp.sk.sign(hash, None)
    } else if clave_bytes.len() == 64 {
        let sk = ed25519_compact::SecretKey::from_slice(&clave_bytes)
            .map_err(|e| format!("clave privada invalida: {e}"))?;
        sk.sign(hash, None)
    } else {
        return Err(format!(
            "la clave privada debe traer 32 (seed) o 64 (SecretKey) bytes; trae {}",
            clave_bytes.len()
        ));
    };

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    out.write_all(firma.as_ref())
        .map_err(|e| format!("no pude escribir la firma a stdout: {e}"))?;
    out.flush().ok();
    Ok(())
}

// =============================================================================
//  FASE 48 :: ceremonia de fianza de claves soberanas
// -----------------------------------------------------------------------------
//  `wawactl claves forjar/derivar-pubkey` es la unica forma legitima de
//  poblar el anillo `AGORA_AUTH_RING` del kernel con claves vivas. Toda la
//  criptografia ocurre AQUI; el kernel jamas ve la seed. La ceremonia es
//  estricta:
//
//    1. Forjar la seed con entropia del SO (lectura cruda de /dev/urandom).
//    2. Derivar el par Ed25519 con `ed25519-compact`.
//    3. Persistir la seed con 0600 — la soberania del operador es fisica.
//    4. Emitir la pubkey como literal de Rust para que el operador la
//       pegue manualmente en `kernel/src/claves.rs::AGORA_AUTH_RING`.
//
//  ZERO heuristica: si el archivo destino ya existe, abortamos sin
//  sobrescribir. Una seed perdida no se recupera; rehusarse a clobberear
//  es elemental.
// =============================================================================

/// Slots validos del anillo. Anclados con la convencion documentada en
/// el kernel: 0=primaria, 1=secundaria, 2=recuperacion.
const SLOTS_VALIDOS: &[u8] = &[0, 1, 2];

/// Lee `N` bytes de `/dev/urandom`. Es la fuente CSPRNG estandar en
/// Linux/BSD/macOS — bloquea solo en el arranque tempranisimo del SO,
/// nunca en una sesion de operador interactivo. Evitamos pulsar el
/// feature `random` de `ed25519-compact` para no inflar la matriz de
/// dependencias del workspace.
fn leer_entropia<const N: usize>() -> Result<[u8; N], String> {
    use std::io::Read;
    let mut f = std::fs::File::open("/dev/urandom")
        .map_err(|e| format!("no pude abrir /dev/urandom: {e}"))?;
    let mut buf = [0u8; N];
    f.read_exact(&mut buf)
        .map_err(|e| format!("entropia del SO truncada: {e}"))?;
    Ok(buf)
}

/// Formatea 32 bytes como un array literal de Rust de cuatro filas de
/// ocho bytes — el estilo que ya usa `AGORA_AUTH_RING` en el kernel.
/// La salida es pegable directa, sin retoques.
fn pubkey_literal_rust(pk: &[u8; 32]) -> String {
    let mut s = String::with_capacity(256);
    s.push_str("[\n");
    for fila in 0..4 {
        s.push_str("    ");
        for col in 0..8 {
            let i = fila * 8 + col;
            s.push_str(&format!("0x{:02x}, ", pk[i]));
        }
        s.push('\n');
    }
    s.push(']');
    s
}

/// Decodifica una clave privada desde un buffer crudo. Acepta 32 B
/// (seed) o 64 B (SecretKey expandida); cualquier otro tamaño aborta.
fn pubkey_de_seed_o_sk(bytes: &[u8]) -> Result<[u8; 32], String> {
    if bytes.len() == 32 {
        let mut seed_arr = [0u8; 32];
        seed_arr.copy_from_slice(bytes);
        let seed = ed25519_compact::Seed::new(seed_arr);
        let kp = ed25519_compact::KeyPair::from_seed(seed);
        let mut pk = [0u8; 32];
        pk.copy_from_slice(kp.pk.as_ref());
        Ok(pk)
    } else if bytes.len() == 64 {
        let sk = ed25519_compact::SecretKey::from_slice(bytes)
            .map_err(|e| format!("clave privada invalida: {e}"))?;
        let mut pk = [0u8; 32];
        pk.copy_from_slice(sk.public_key().as_ref());
        Ok(pk)
    } else {
        Err(format!(
            "la clave privada debe traer 32 (seed) o 64 (SecretKey) bytes; trae {}",
            bytes.len()
        ))
    }
}

/// `wawactl claves forjar --slot N --salida PATH`. Genera un par
/// Ed25519 fresco, persiste la seed con 0600 e imprime la pubkey como
/// array literal de Rust. Aborta si el archivo destino ya existe — un
/// re-forjado accidental sobre una seed viva equivaldria a perder la
/// identidad del operador en ese slot.
fn cmd_claves_forjar(slot: u8, salida: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    if !SLOTS_VALIDOS.contains(&slot) {
        return Err(format!(
            "slot fuera de rango: {slot}. Validos: 0 (primaria), 1 (secundaria), 2 (recuperacion)"
        ));
    }

    // Aforjamos la seed con entropia del SO ANTES de tocar el disco.
    // Si /dev/urandom falla, no dejamos un archivo a medio escribir.
    let mut seed_bytes: [u8; 32] = leer_entropia()?;
    let seed = ed25519_compact::Seed::new(seed_bytes);
    let kp = ed25519_compact::KeyPair::from_seed(seed);
    let mut pk_bytes = [0u8; 32];
    pk_bytes.copy_from_slice(kp.pk.as_ref());

    // Persistir con `create_new` (E EXCL) + mode 0600. Cualquier
    // colision con un archivo preexistente aborta sin clobberear.
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(salida)
        .map_err(|e| format!("no pude crear {salida} (0600, exclusivo): {e}"))?;
    f.write_all(&seed_bytes)
        .map_err(|e| format!("no pude escribir la seed en {salida}: {e}"))?;
    f.sync_all().ok();
    drop(f);

    // Limpieza absoluta del buffer en memoria antes de salir de scope.
    // No es defensa criptografica de la mas pura — el kernel pudo haber
    // copiado la pagina a swap antes — pero minimiza la huella en la
    // memoria del proceso. Equivalente moral al `mlock` + zeroize que
    // un HSM serio haria.
    seed_bytes.fill(0);

    let etiqueta = match slot {
        0 => "PRIMARIA",
        1 => "SECUNDARIA",
        2 => "RECUPERACION",
        _ => unreachable!(),
    };
    println!("// SLOT {slot} ({etiqueta}) PUBLIC KEY LITERAL");
    println!("// Seed persistida en: {salida}");
    println!("{},", pubkey_literal_rust(&pk_bytes));
    Ok(())
}

/// `wawactl claves derivar-pubkey --clave-privada PATH`. Carga la
/// seed/SecretKey persistida, recalcula su pubkey y la emite como
/// literal Rust. Util como auditoria forense: el operador verifica
/// que la pubkey grabada en el kernel sigue casando con la seed que
/// guarda offline antes de aceptar una nueva imagen.
fn cmd_claves_derivar_pubkey(clave_path: &str) -> Result<(), String> {
    let mut clave_bytes = std::fs::read(clave_path)
        .map_err(|e| format!("no pude leer la clave privada en {clave_path}: {e}"))?;
    let pk = pubkey_de_seed_o_sk(&clave_bytes)?;
    clave_bytes.fill(0); // ver `cmd_claves_forjar` para el porque.
    println!("// PUBLIC KEY DERIVADA DE {clave_path}");
    println!("{},", pubkey_literal_rust(&pk));
    Ok(())
}

fn layer_of(system: bool) -> Layer {
    if system { Layer::System } else { Layer::User }
}

fn layer_label(l: Layer) -> &'static str {
    match l {
        Layer::System => "system",
        Layer::User => "user",
    }
}

fn cmd_path(layer: Layer) -> Result<(), String> {
    let path = WawaConfig::path_for(layer)
        .ok_or_else(|| format!("la capa {} no aplica en esta plataforma", layer_label(layer)))?;
    println!("{}", path.display());
    Ok(())
}

fn cmd_show(json: bool, layer: ShowLayer) -> Result<(), String> {
    let cfg = match layer {
        ShowLayer::Effective => WawaConfig::load(),
        ShowLayer::System => WawaConfig::load_layer(Layer::System).ok_or_else(|| {
            "capa de sistema ausente (no existe /etc/wawa/config.json)".to_string()
        })?,
        ShowLayer::User => WawaConfig::load_layer(Layer::User)
            .ok_or_else(|| "capa de usuario ausente".to_string())?,
    };
    let s = if json {
        serde_json::to_string(&cfg).map_err(|e| e.to_string())?
    } else {
        serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?
    };
    println!("{s}");
    Ok(())
}

fn cmd_get(key: &str) -> Result<(), String> {
    let cfg = WawaConfig::load();
    let val = match key {
        "theme_variant" => cfg.theme_variant,
        "accent" => cfg.accent,
        "lang" => cfg.lang,
        "timefmt_24h" => cfg.timefmt_24h.to_string(),
        _ => return Err(format!("key desconocida: {key}")),
    };
    println!("{val}");
    Ok(())
}

fn cmd_set(key: &str, value: &str, layer: Layer) -> Result<(), String> {
    // Para set/reset/module sobre una capa, tomamos como base la
    // config **de esa capa** (si existe) — no la efectiva — para no
    // promover valores del usuario al archivo de sistema (o
    // viceversa) por accidente.
    let mut cfg = WawaConfig::load_layer(layer).unwrap_or_default();
    match key {
        "theme_variant" => {
            validate_theme(value)?;
            cfg.theme_variant = value.into();
        }
        "accent" => cfg.accent = value.into(),
        "lang" => {
            validate_lang(value)?;
            cfg.lang = value.into();
        }
        "timefmt_24h" => {
            cfg.timefmt_24h = parse_bool_or_clock(value)?;
        }
        _ => return Err(format!("key desconocida: {key}")),
    }
    let path = cfg.save_to(layer).map_err(|e| e.to_string())?;
    println!(
        "✓ [{}] {} = {}  →  {}",
        layer_label(layer),
        key,
        value,
        path.display()
    );
    Ok(())
}

fn cmd_reset(layer: Layer) -> Result<(), String> {
    let cfg = WawaConfig::default();
    let path = cfg.save_to(layer).map_err(|e| e.to_string())?;
    println!("✓ [{}] reset  →  {}", layer_label(layer), path.display());
    Ok(())
}

fn cmd_module(id: &str, op: &str, layer: Layer) -> Result<(), String> {
    let mut cfg = WawaConfig::load_layer(layer).unwrap_or_default();
    let before = cfg.module_enabled(id);
    let after = match op {
        "on" => true,
        "off" => false,
        "toggle" => !before,
        other => return Err(format!("op inválida: {other} (usar on|off|toggle)")),
    };
    cfg.modules.insert(id.into(), after);
    let path = cfg.save_to(layer).map_err(|e| e.to_string())?;
    println!(
        "✓ [{}] module {} = {}  ({})  →  {}",
        layer_label(layer),
        id,
        if after { "on" } else { "off" },
        if before == after { "sin cambios" } else { "cambiado" },
        path.display(),
    );
    Ok(())
}

fn cmd_watch() -> Result<(), String> {
    let user = WawaConfig::path_for(Layer::User).ok_or("no hay ProjectDirs")?;
    match WawaConfig::path_for(Layer::System) {
        Some(sys) => println!("watching {} y {}…", sys.display(), user.display()),
        None => println!("watching {}…", user.display()),
    }
    let (tx, rx) = mpsc::channel::<WawaConfig>();
    let _watcher = ConfigWatcher::spawn(move |cfg| {
        let _ = tx.send(cfg);
    })
    .map_err(|e| format!("watcher: {e}"))?;

    // Imprimimos la versión inicial como ancla.
    let initial = WawaConfig::load();
    print_change(&initial, true);

    let mut last = initial;
    loop {
        match rx.recv_timeout(Duration::from_secs(60 * 60 * 24)) {
            Ok(cfg) => {
                if cfg != last {
                    print_change(&cfg, false);
                    last = cfg;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn print_change(cfg: &WawaConfig, initial: bool) {
    let prefix = if initial { "·" } else { "↻" };
    let json = serde_json::to_string(cfg).unwrap_or_else(|_| String::from("?"));
    println!("{prefix} {json}");
}

// =====================================================================
// Validadores
// =====================================================================

fn validate_theme(v: &str) -> Result<(), String> {
    match v {
        "dark" | "light" | "aurora" | "sunset" => Ok(()),
        other => Err(format!(
            "theme_variant inválido: {other} (usar dark|light|aurora|sunset)"
        )),
    }
}

fn validate_lang(v: &str) -> Result<(), String> {
    match v {
        "es-PE" | "en-US" | "qu-PE" => Ok(()),
        other => Err(format!(
            "lang inválido: {other} (usar es-PE|en-US|qu-PE)"
        )),
    }
}

fn parse_bool_or_clock(v: &str) -> Result<bool, String> {
    match v {
        "true" | "24" | "24h" => Ok(true),
        "false" | "12" | "12h" => Ok(false),
        other => Err(format!("bool inválido: {other}")),
    }
}

// =============================================================================
//  FASE 39 :: demonio bidireccional de firma del cuaderno
// -----------------------------------------------------------------------------
//  El subcomando `wawactl daemon-firma --pty <path> --clave-privada <path>`
//  vive aqui. Lazo asincrono basado en tokio, escucha del PTY de QEMU
//  (mapeado a COM1 de Wawa por `-serial pty`), parser tolerante a basura,
//  confirmacion interactiva del operador humano con timeout 30 s, y log de
//  auditoria en `wawactl_audit.log`.
// =============================================================================

/// LEGACY (Fase 38/39) :: prefijo ASCII emitido por el kernel a traves
/// del UART de COM1 antes del hash en hexadecimal + newline. Aplica a
/// solicitudes de firma de CUADERNO (Fase 39) y MANIFIESTO (Fase 41) —
/// ambos comparten esta puerta ASCII por simetria historica.
const PREFIJO_SOLICITUD_ASCII: &str = "wawa::sign_request::";
/// FASE 60 v2 :: prefijo ASCII paralelo para solicitudes de firma de
/// CONFIGURACION. La firma criptografica es identica (Ed25519 sobre el
/// hash) pero el prompt y el log de auditoria distinguen el tipo, de modo
/// que la cadena "el operador firmo X" sea trazeable hasta qué tipo de
/// objeto.
const PREFIJO_SOLICITUD_CONFIG_ASCII: &str = "wawa::sign_config::";
/// FASE 49 :: prefijo BINARIO emitido por el kernel a traves del bus
/// virtio-console antes de los 32 bytes RAW del hash. Sin newline; el
/// parser mide por longitud fija (19 prefijo + 32 hash = 51 bytes).
const PREFIJO_SOLICITUD_VIRTIO: &[u8] = b"wawactl::sign_pci::";
/// FASE 60 v2 :: prefijo BINARIO paralelo para solicitudes de firma de
/// CONFIGURACION sobre virtio-console. Mismo largo (19 B) que el de
/// cuaderno para que la ventana deslizante del parser binario no cambie
/// de tamano — la unica diferencia es el discriminante.
const PREFIJO_SOLICITUD_CONFIG_VIRTIO: &[u8] = b"wawactl::sign_cfg::";
/// Longitud del hash en caracteres hexadecimales (32 bytes -> 64 chars).
const HASH_HEX_LEN: usize = 64;

/// FASE 60 v2 :: tipo del objeto que el kernel pide firmar. Mismo
/// algoritmo criptografico, distinto SIGNIFICADO — el operador ve el tipo
/// en el prompt y la auditoria queda taggeada para que el log sea legible
/// a posteriori. Si manana se agrega un tercer tipo (estados de app, sello
/// de canal, etc.) se suma una variante aqui sin tocar el resto del flujo.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TipoSolicitud {
    /// Cuaderno o manifiesto. Heredados de las fases 39 y 41 — usan el
    /// prefijo `wawa::sign_request::` / `wawactl::sign_pci::`.
    Cuaderno,
    /// Configuracion del sistema (Fase 60). Usa los prefijos
    /// `wawa::sign_config::` / `wawactl::sign_cfg::`.
    Configuracion,
}

impl TipoSolicitud {
    /// Etiqueta corta para el log de auditoria. Estable — los lectores
    /// del log pueden grep-ear por ella.
    fn etiqueta_audit(self) -> &'static str {
        match self {
            TipoSolicitud::Cuaderno => "cuaderno",
            TipoSolicitud::Configuracion => "configuracion",
        }
    }

    /// Etiqueta del prompt interactivo — visible al operador antes de que
    /// pulse y/N. Lo que se le muestra es el TIPO del objeto, no su
    /// algoritmo (que es siempre el mismo).
    fn etiqueta_prompt(self) -> &'static str {
        match self {
            TipoSolicitud::Cuaderno => "CUADERNO/MANIFIESTO",
            TipoSolicitud::Configuracion => "CONFIGURACION",
        }
    }
}

/// FASE 49 :: transporte fisico del demonio. Selecciona prefijo, parser
/// y forma de lectura (lineas ASCII vs ventana binaria).
#[derive(Clone, Copy)]
enum ModoTransporte {
    /// UART legacy via PTY (Fase 38/39). Parser ASCII; lectura por
    /// lineas terminadas en `\n`. Bandera CLI: `--pty`.
    PtyAscii,
    /// VirtIO Console via char device (Fase 49). Parser binario;
    /// lectura por ventana deslizante. Bandera CLI: `--char-device`
    /// o `--virtio-port`.
    VirtioBinario,
}
/// Tiempo maximo que el demonio espera la confirmacion del operador.
/// 30 s alinea con la cadencia humana sin congelar el kernel — si no
/// hay decision, la syscall en Wawa expira con `Saturado` y la app
/// puede reintentar despues.
const TIMEOUT_CONFIRMACION: Duration = Duration::from_secs(30);

#[derive(Clone, Copy)]
enum DecisionOperador {
    /// El operador autorizo. Firmamos y devolvemos los 64 bytes raw.
    Autorizada,
    /// FASE 41 :: la solicitud cayo dentro de la ventana de
    /// pre-autorizacion temporal. Firmamos sin prompt; auditamos con
    /// el marcador explicito `FIRMA_AUTO_EMITIDA`.
    AutorizadaAutomatica,
    /// El operador rechazo o el timeout expiro. No firmamos; el kernel
    /// vera `Saturado` y la app puede reintentar.
    Rechazada,
}

/// FASE 39/41/49 :: subcomando `daemon-firma`. Crea un runtime tokio y
/// delega en el ejecutor que case con el `ModoTransporte`. Devolver
/// `Err` aqui imprime al stderr de wawactl y sale con codigo no-cero.
/// FASE 63 :: lo que el host emite por el virtio-console para pedir un GC.
/// Terminado en '\n' — el lado kernel (`control.rs`) parsea por lineas.
const GC_REQUEST: &[u8] = b"wawactl::gc_request::\n";

/// FASE 63 :: prefijo de la respuesta del kernel. El cuerpo es
/// `vivos=N muertos=M sectores=A->B` o `error::<motivo>`.
const GC_REPLY_PREFIJO: &[u8] = b"wawactl::gc_reply::";

/// FASE 63 :: subcomando `wawactl gc`. Abre el virtio-console, emite el
/// `gc_request` y espera la linea de veredicto con timeout. Operacion de un
/// solo disparo: ni demonio ni estado persistente.
fn cmd_gc(transporte_path: &str, timeout_s: u64) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("no pude construir el runtime tokio: {e}"))?;
    rt.block_on(ejecutar_gc(transporte_path.to_string(), timeout_s))
}

async fn ejecutar_gc(transporte_path: String, timeout_s: u64) -> Result<(), String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let file = tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&transporte_path)
        .await
        .map_err(|e| format!("no pude abrir {transporte_path}: {e}"))?;
    let (mut reader, mut writer) = tokio::io::split(file);

    writer
        .write_all(GC_REQUEST)
        .await
        .map_err(|e| format!("no pude emitir gc_request: {e}"))?;
    writer.flush().await.ok();
    eprintln!("wawactl: gc_request emitido a {transporte_path}; esperando veredicto…");

    // Acumular el RX y escanear linea a linea hasta hallar el reply. La
    // ventana tolera basura previa (trazas de boot, ecos del propio request).
    let mut acc: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 256];
    let leer = async {
        loop {
            let n = reader
                .read(&mut chunk)
                .await
                .map_err(|e| format!("error leyendo del transporte: {e}"))?;
            if n == 0 {
                return Err("transporte cerrado antes del veredicto".to_string());
            }
            acc.extend_from_slice(&chunk[..n]);
            if let Some(linea) = extraer_gc_reply(&acc) {
                return Ok(linea);
            }
        }
    };

    match tokio::time::timeout(std::time::Duration::from_secs(timeout_s), leer).await {
        Ok(Ok(linea)) => {
            println!("{linea}");
            Ok(())
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err(format!(
            "timeout: el kernel no respondio en {timeout_s} s \
             (¿vivo el huesped? ¿es el char device correcto?)"
        )),
    }
}

/// Escanea `buf` linea a linea (terminadas en '\n') buscando la que empiece
/// por `wawactl::gc_reply::`. Devuelve su contenido sin el salto de linea.
/// `None` si aun no llego una linea de reply completa.
fn extraer_gc_reply(buf: &[u8]) -> Option<String> {
    let mut inicio = 0;
    while inicio < buf.len() {
        let fin = buf[inicio..].iter().position(|&b| b == b'\n')? + inicio;
        let linea = &buf[inicio..fin];
        if linea.starts_with(GC_REPLY_PREFIJO) {
            return Some(String::from_utf8_lossy(linea).trim_end().to_string());
        }
        inicio = fin + 1;
    }
    None
}

fn cmd_daemon_firma(
    transporte: &str,
    modo: ModoTransporte,
    clave_path: &str,
    log_path: &str,
    auto_firmar_durante: Option<&str>,
    slot: u8,
) -> Result<(), String> {
    // FASE 42 :: validar el slot ANTES de hacer cualquier I/O. Tres slots
    // legitimos: 0 primaria, 1 secundaria, 2 recuperacion. Cualquier
    // otro valor es un error de invocacion del usuario.
    if slot > 2 {
        return Err(format!(
            "--slot {slot}: el anillo AGORA_AUTH_RING tiene 3 slots (0/1/2)"
        ));
    }
    // Cargar la clave privada UNA SOLA VEZ al arrancar — el ataque
    // superficie del lazo asincrono no la toca a partir de ahi.
    let sk = cargar_clave_privada(clave_path)?;

    // FASE 41 :: parsear la ventana de pre-autorizacion (si fue indicada)
    // y traducirla a un `Instant` de expiracion. La ventana se computa
    // una sola vez al arranque del demonio — el reloj monotono del
    // sistema garantiza que no haya "salto hacia atras" si la hora del
    // SO cambia (NTP, etc.).
    let ventana_hasta = match auto_firmar_durante {
        Some(s) => Some(
            std::time::Instant::now()
                + parse_duracion(s).map_err(|e| format!("--auto-firmar-durante: {e}"))?,
        ),
        None => None,
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("no pude construir el runtime tokio: {e}"))?;
    match modo {
        ModoTransporte::PtyAscii => rt.block_on(ejecutar_daemon(
            transporte.to_string(),
            sk,
            log_path.to_string(),
            ventana_hasta,
            slot,
        )),
        ModoTransporte::VirtioBinario => rt.block_on(ejecutar_daemon_virtio(
            transporte.to_string(),
            sk,
            log_path.to_string(),
            ventana_hasta,
            slot,
        )),
    }
}

/// FASE 41 :: parser de duraciones humanas (`30s`, `15m`, `1h`). Sin alocacion,
/// sin deps externas. Acepta numeros enteros positivos seguidos de unidad.
fn parse_duracion(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("vacio".to_string());
    }
    let (numero, unidad) = s.split_at(s.len() - 1);
    let n: u64 = numero
        .parse()
        .map_err(|_| format!("`{s}` no es un numero seguido de unidad (s/m/h)"))?;
    let multiplicador = match unidad {
        "s" => 1u64,
        "m" => 60,
        "h" => 3600,
        otro => return Err(format!("unidad desconocida `{otro}` (esperaba s/m/h)")),
    };
    Ok(Duration::from_secs(
        n.checked_mul(multiplicador)
            .ok_or_else(|| format!("duracion `{s}` desborda u64"))?,
    ))
}

/// Carga la clave privada desde un archivo. Acepta dos formatos:
///   * 32 bytes -> seed (ed25519-compact deriva el keypair).
///   * 64 bytes -> SecretKey "expanded" canonica.
fn cargar_clave_privada(path: &str) -> Result<ed25519_compact::SecretKey, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("no pude leer la clave privada en {path}: {e}"))?;
    match bytes.len() {
        32 => {
            let mut seed = [0u8; 32];
            seed.copy_from_slice(&bytes);
            let kp = ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new(seed));
            Ok(kp.sk)
        }
        64 => ed25519_compact::SecretKey::from_slice(&bytes)
            .map_err(|e| format!("SecretKey invalida: {e}")),
        n => Err(format!(
            "la clave debe traer 32 (seed) o 64 (SecretKey) bytes; trae {n}"
        )),
    }
}

/// Lazo asincrono del demonio. Mantiene un BufReader sobre la mitad de
/// lectura del PTY y reescribe en la mitad de escritura los 64 bytes
/// raw de cada firma autorizada.
async fn ejecutar_daemon(
    pty_path: String,
    sk: ed25519_compact::SecretKey,
    log_path: String,
    ventana_hasta: Option<std::time::Instant>,
    slot: u8,
) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    eprintln!("wawactl: daemon-firma escuchando en {pty_path}");
    eprintln!("wawactl: auditoria a {log_path}");
    eprintln!("wawactl: firmando con slot {slot} del anillo AGORA_AUTH_RING");
    if let Some(t) = ventana_hasta {
        let restante = t.saturating_duration_since(std::time::Instant::now());
        eprintln!(
            "wawactl: ventana de auto-firma activa durante {} s",
            restante.as_secs()
        );
    }
    eprintln!("wawactl: Ctrl-C para terminar");

    let file = tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&pty_path)
        .await
        .map_err(|e| format!("no pude abrir {pty_path}: {e}"))?;
    // Split del File en R+W. `tokio::io::split` requiere AsyncRead + AsyncWrite;
    // `tokio::fs::File` cumple ambos rasgos por separado en sus halves.
    let (reader, mut writer) = tokio::io::split(file);
    let mut reader = BufReader::new(reader);
    let mut linea = String::new();

    loop {
        linea.clear();
        let n = reader
            .read_line(&mut linea)
            .await
            .map_err(|e| format!("error leyendo PTY: {e}"))?;
        if n == 0 {
            eprintln!("wawactl: PTY cerrada — saliendo");
            return Ok(());
        }

        // El parser es ESTRICTO: la linea debe arrancar con uno de los
        // prefijos conocidos y traer 64 chars hex justo despues. Cualquier
        // otra basura (logs del kernel, trazas de boot) cae al sumidero
        // silencioso — el operador la ve en su terminal de QEMU igual.
        let trim = linea.trim_end_matches(|c| c == '\n' || c == '\r');
        // FASE 60 v2 :: el parser detecta ambos prefijos (cuaderno/config)
        // y tagguea el tipo para que el prompt y el log distingan.
        let Some((tipo, hash_hex)) = clasificar_linea_ascii(trim) else {
            continue;
        };
        if hash_hex.len() != HASH_HEX_LEN || !hash_hex.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let mut hash = [0u8; 32];
        if !hex_a_bytes(hash_hex, &mut hash) {
            continue;
        }

        // FASE 41 :: ventana de pre-autorizacion. Si el reloj monotono cae
        // dentro de la ventana, la solicitud se firma sin prompt; al
        // expirar la ventana volvemos al modo interactivo restrictivo.
        let dentro_ventana = ventana_hasta
            .map(|t| std::time::Instant::now() < t)
            .unwrap_or(false);
        let decision = if dentro_ventana {
            DecisionOperador::AutorizadaAutomatica
        } else {
            confirmar_con_operador(tipo, hash_hex).await
        };

        match decision {
            DecisionOperador::Autorizada | DecisionOperador::AutorizadaAutomatica => {
                let marcador = match decision {
                    DecisionOperador::AutorizadaAutomatica => "FIRMA_AUTO_EMITIDA",
                    _ => "FIRMA_EMITIDA",
                };
                let firma = sk.sign(hash, None);
                let bytes = firma.as_ref();
                // FASE 42 :: el frame inyectado al kernel ahora es de 65 B:
                // byte 0 = slot del anillo AGORA_AUTH_RING, bytes 1..65 =
                // los 64 bytes crudos del sello Ed25519. La app enjaulada
                // autodetecta que clave publica embeber en el sobre
                // CuadernoFirmado leyendo el byte 0.
                let mut frame = [0u8; 65];
                frame[0] = slot;
                frame[1..65].copy_from_slice(bytes);
                writer
                    .write_all(&frame)
                    .await
                    .map_err(|e| format!("error escribiendo frame al PTY: {e}"))?;
                writer
                    .flush()
                    .await
                    .map_err(|e| format!("error flush PTY: {e}"))?;
                escribir_auditoria(&log_path, marcador, tipo, slot, hash_hex);
                eprintln!(
                    "wawactl: {} tipo={} slot={} ({} bytes incluyendo prefijo)",
                    marcador,
                    tipo.etiqueta_audit(),
                    slot,
                    frame.len()
                );
            }
            DecisionOperador::Rechazada => {
                escribir_auditoria(&log_path, "FIRMA_RECHAZADA", tipo, slot, hash_hex);
                eprintln!("wawactl: solicitud rechazada o timeout");
            }
        }
    }
}

/// FASE 49 :: lazo asincrono del demonio sobre el bus virtio-console.
/// El parser ya no es por lineas: el kernel emite 19 B de prefijo +
/// 32 B raw de hash, sin separador. Usamos una ventana deslizante de
/// bytes que detecta el prefijo y luego absorbe los 32 bytes
/// siguientes como hash crudo. La respuesta sigue siendo los 65 B de
/// `[slot | firma]` —contrato del Userspace inalterado.
async fn ejecutar_daemon_virtio(
    transporte_path: String,
    sk: ed25519_compact::SecretKey,
    log_path: String,
    ventana_hasta: Option<std::time::Instant>,
    slot: u8,
) -> Result<(), String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    eprintln!("wawactl: daemon-firma escuchando en {transporte_path} (virtio-console)");
    eprintln!("wawactl: auditoria a {log_path}");
    eprintln!("wawactl: firmando con slot {slot} del anillo AGORA_AUTH_RING");
    if let Some(t) = ventana_hasta {
        let restante = t.saturating_duration_since(std::time::Instant::now());
        eprintln!(
            "wawactl: ventana de auto-firma activa durante {} s",
            restante.as_secs()
        );
    }
    eprintln!("wawactl: Ctrl-C para terminar");

    let file = tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&transporte_path)
        .await
        .map_err(|e| format!("no pude abrir {transporte_path}: {e}"))?;
    let (mut reader, mut writer) = tokio::io::split(file);

    // Ventana deslizante. Mantiene como mucho los ultimos `pref + 32`
    // bytes; cuando alguno de los prefijos aparece pegado al final, los
    // 32 siguientes forman el hash. Sin alocacion dinamica — un array
    // estatico de `FRAME_LEN` bytes con cursor manual basta.
    //
    // FASE 60 v2 :: dos prefijos del mismo largo (cuaderno y configuracion)
    // se chequean en cada paso; la ventana es indiferente al tipo. Esto
    // requiere que ambos `PREFIJO_SOLICITUD_*_VIRTIO` midan 19 bytes —
    // si alguno cambia, ajustar aqui.
    const PREF_LEN: usize = PREFIJO_SOLICITUD_VIRTIO.len();
    const FRAME_LEN: usize = PREF_LEN + 32;
    debug_assert_eq!(
        PREFIJO_SOLICITUD_VIRTIO.len(),
        PREFIJO_SOLICITUD_CONFIG_VIRTIO.len(),
        "los dos prefijos binarios deben medir lo mismo para que la ventana sea compartida"
    );
    let mut ventana = [0u8; FRAME_LEN];
    let mut llenos: usize = 0;
    let mut chunk = [0u8; 256];

    loop {
        let n = reader
            .read(&mut chunk)
            .await
            .map_err(|e| format!("error leyendo del transporte virtio: {e}"))?;
        if n == 0 {
            eprintln!("wawactl: transporte cerrado — saliendo");
            return Ok(());
        }
        for &b in &chunk[..n] {
            // Avanzar el cursor; si la ventana se llena sin haber
            // matcheado el prefijo, desplazamos un byte a la izquierda
            // — el parser tolera basura intercalada (bytes de boot,
            // trazas) sin perder un prefijo legitimo.
            if llenos < FRAME_LEN {
                ventana[llenos] = b;
                llenos += 1;
            } else {
                ventana.copy_within(1..FRAME_LEN, 0);
                ventana[FRAME_LEN - 1] = b;
            }
            // Buscar uno de los prefijos en offset 0 — solo ese match
            // deja 32 bytes utiles a su derecha.
            if llenos != FRAME_LEN {
                continue;
            }
            let tipo = match &ventana[..PREF_LEN] {
                p if p == PREFIJO_SOLICITUD_VIRTIO => Some(TipoSolicitud::Cuaderno),
                p if p == PREFIJO_SOLICITUD_CONFIG_VIRTIO => Some(TipoSolicitud::Configuracion),
                _ => None,
            };
            let Some(tipo) = tipo else {
                continue;
            };
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&ventana[PREF_LEN..]);
            let hash_hex = hex_de_bytes(&hash);

            let dentro_ventana = ventana_hasta
                .map(|t| std::time::Instant::now() < t)
                .unwrap_or(false);
            let decision = if dentro_ventana {
                DecisionOperador::AutorizadaAutomatica
            } else {
                confirmar_con_operador(tipo, &hash_hex).await
            };

            match decision {
                DecisionOperador::Autorizada | DecisionOperador::AutorizadaAutomatica => {
                    let marcador = match decision {
                        DecisionOperador::AutorizadaAutomatica => "FIRMA_AUTO_EMITIDA",
                        _ => "FIRMA_EMITIDA",
                    };
                    let firma = sk.sign(hash, None);
                    let mut frame = [0u8; 65];
                    frame[0] = slot;
                    frame[1..65].copy_from_slice(firma.as_ref());
                    writer
                        .write_all(&frame)
                        .await
                        .map_err(|e| format!("error escribiendo frame al transporte: {e}"))?;
                    writer
                        .flush()
                        .await
                        .map_err(|e| format!("error flush transporte: {e}"))?;
                    escribir_auditoria(&log_path, marcador, tipo, slot, &hash_hex);
                    eprintln!(
                        "wawactl: {} tipo={} slot={} (65 B sobre virtio-console)",
                        marcador,
                        tipo.etiqueta_audit(),
                        slot
                    );
                }
                DecisionOperador::Rechazada => {
                    escribir_auditoria(&log_path, "FIRMA_RECHAZADA", tipo, slot, &hash_hex);
                    eprintln!("wawactl: solicitud rechazada o timeout");
                }
            }
            // Reset de la ventana: un nuevo prefijo arrancara desde
            // cero. Sin esto un mismo bloque podria gatillar dos
            // veces si el contenido casa fortuitamente.
            llenos = 0;
        }
    }
}

/// Hex-encode de un hash de 32 bytes a 64 chars ASCII. Util para el
/// prompt interactivo y el log de auditoria — el operador lee hex,
/// el bus mueve binario.
fn hex_de_bytes(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// Muestra el prompt al operador y lee `y`/`N` por stdin con timeout. La
/// lectura de stdin es bloqueante (no hay un read async robusto para
/// stdin en tokio sin features adicionales); usamos `spawn_blocking`
/// para que el lazo principal pueda imponer el timeout via
/// `tokio::time::timeout`.
///
/// FASE 60 v2 :: el prompt declara el TIPO del objeto (cuaderno/manifiesto
/// vs configuracion) para que el operador pueda decidir con contexto. El
/// algoritmo de firma es el mismo (Ed25519 sobre el hash), pero el
/// significado del acto cambia.
async fn confirmar_con_operador(tipo: TipoSolicitud, hash_hex: &str) -> DecisionOperador {
    use std::io::Write;
    eprintln!();
    eprintln!("================================================================");
    eprintln!("  SOLICITUD DE FIRMA DE {}", tipo.etiqueta_prompt());
    eprintln!("  HASH: {hash_hex}");
    eprintln!("  Autorizar firma en el metal? [y/N]  (timeout 30 s)");
    eprintln!("================================================================");
    let _ = std::io::stderr().flush();

    let respuesta = tokio::task::spawn_blocking(|| {
        let mut buf = String::new();
        let _ = std::io::stdin().read_line(&mut buf);
        buf.trim().to_string()
    });

    match tokio::time::timeout(TIMEOUT_CONFIRMACION, respuesta).await {
        Ok(Ok(s)) if s.eq_ignore_ascii_case("y") => DecisionOperador::Autorizada,
        Ok(_) => DecisionOperador::Rechazada,
        Err(_) => {
            eprintln!("wawactl: timeout — sin respuesta del operador");
            DecisionOperador::Rechazada
        }
    }
}

/// FASE 60 v2 :: clasifica una linea ASCII contra los prefijos conocidos.
/// Devuelve `(tipo, hash_hex)` si la linea arranca con uno de los
/// prefijos legitimos, o `None` si es basura (logs del kernel, trazas
/// de boot). El llamante valida que `hash_hex` sea 64 chars hex; aqui
/// solo separamos el discriminante del payload.
fn clasificar_linea_ascii(linea: &str) -> Option<(TipoSolicitud, &str)> {
    if let Some(hex) = linea.strip_prefix(PREFIJO_SOLICITUD_ASCII) {
        return Some((TipoSolicitud::Cuaderno, hex));
    }
    if let Some(hex) = linea.strip_prefix(PREFIJO_SOLICITUD_CONFIG_ASCII) {
        return Some((TipoSolicitud::Configuracion, hex));
    }
    None
}

/// Convierte una cadena hexadecimal de 64 chars a `[u8; 32]`. Devuelve
/// `false` si algun caracter no es hex valido — el parser de lineas ya
/// filtro antes, pero defensa en profundidad no cuesta.
fn hex_a_bytes(hex: &str, out: &mut [u8; 32]) -> bool {
    if hex.len() != 64 {
        return false;
    }
    for i in 0..32 {
        match u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16) {
            Ok(b) => out[i] = b,
            Err(_) => return false,
        }
    }
    true
}

/// Append a `log_path` de una entrada estructurada de auditoria. Cada
/// linea contiene timestamp ISO 8601, accion (FIRMA_EMITIDA /
/// FIRMA_AUTO_EMITIDA / FIRMA_RECHAZADA), TIPO del objeto firmado
/// (cuaderno/configuracion), slot del anillo AGORA_AUTH_RING con que se
/// firmo, y el hash. Errores de I/O se imprimen a stderr pero NO
/// interrumpen el lazo — perder una linea de log es preferible a perder
/// el demonio entero. FASE 42 :: el campo `SLOT` distingue las firmas
/// emitidas por distintos dispositivos del operador (primario, secundario,
/// recuperacion). FASE 60 v2 :: el campo `TIPO` permite reconstruir, a
/// posteriori, que clase de objeto firmo cada slot en cada momento.
fn escribir_auditoria(log_path: &str, accion: &str, tipo: TipoSolicitud, slot: u8, hash_hex: &str) {
    use std::io::Write;
    let ts = chrono::Utc::now().to_rfc3339();
    let linea = format!(
        "[{ts}] | ACCION: {accion} | TIPO: {} | SLOT: {slot} | HASH: {hash_hex} | AUTOR: AGORA_AUTH_RING[{slot}]\n",
        tipo.etiqueta_audit(),
    );
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        Ok(mut f) => {
            if let Err(e) = f.write_all(linea.as_bytes()) {
                eprintln!("wawactl: no pude escribir audit log: {e}");
            }
        }
        Err(e) => eprintln!("wawactl: no pude abrir audit log {log_path}: {e}"),
    }
}

// ---------------------------------------------------------------------
// Tests del clasificador de prefijos (Fase 60 v2)
// ---------------------------------------------------------------------

#[cfg(test)]
mod pruebas_clasificador {
    use super::*;

    #[test]
    fn ascii_clasifica_cuaderno() {
        let hash = "a".repeat(64);
        let linea = format!("{PREFIJO_SOLICITUD_ASCII}{hash}");
        let (tipo, hex) = clasificar_linea_ascii(&linea).expect("debe matchear");
        assert_eq!(tipo, TipoSolicitud::Cuaderno);
        assert_eq!(hex, hash);
    }

    #[test]
    fn ascii_clasifica_configuracion() {
        let hash = "b".repeat(64);
        let linea = format!("{PREFIJO_SOLICITUD_CONFIG_ASCII}{hash}");
        let (tipo, hex) = clasificar_linea_ascii(&linea).expect("debe matchear");
        assert_eq!(tipo, TipoSolicitud::Configuracion);
        assert_eq!(hex, hash);
    }

    #[test]
    fn ascii_basura_es_none() {
        assert!(clasificar_linea_ascii("kernel :: trazza random").is_none());
        assert!(clasificar_linea_ascii("").is_none());
        // Prefijo cercano pero distinto al de cuaderno.
        assert!(clasificar_linea_ascii("wawa::sign_wrong::deadbeef").is_none());
    }

    #[test]
    fn ascii_prefijo_sin_hash_devuelve_str_vacio() {
        // Match del prefijo pero sin hex despues. El clasificador ya
        // devuelve `Some("")`; el validador del lazo principal rechaza
        // porque `hex.len() != 64`.
        let (tipo, hex) = clasificar_linea_ascii(PREFIJO_SOLICITUD_ASCII).expect("matchea");
        assert_eq!(tipo, TipoSolicitud::Cuaderno);
        assert!(hex.is_empty());
    }

    #[test]
    fn prefijos_virtio_miden_lo_mismo() {
        // El parser binario asume que ambos prefijos miden igual para
        // poder reusar la ventana deslizante. Si alguien los modifica
        // sin mantener la simetria, este test lo caza.
        assert_eq!(
            PREFIJO_SOLICITUD_VIRTIO.len(),
            PREFIJO_SOLICITUD_CONFIG_VIRTIO.len(),
            "los dos prefijos binarios deben medir lo mismo",
        );
    }

    #[test]
    fn etiqueta_audit_distingue_tipos() {
        // Los lectores del log dependen de strings estables — si alguien
        // los renombra, este test lo caza.
        assert_eq!(TipoSolicitud::Cuaderno.etiqueta_audit(), "cuaderno");
        assert_eq!(
            TipoSolicitud::Configuracion.etiqueta_audit(),
            "configuracion"
        );
    }

    #[test]
    fn gc_reply_extrae_veredicto_entre_basura() {
        // El kernel emite trazas de boot y la baliza serial intercaladas;
        // el parser debe pescar la linea de reply sin tropezar.
        let flujo = b"renaser :: boot ok\ngc :: remoto\n\
                      wawactl::gc_reply::vivos=42 muertos=7 sectores=100->58\nmas ruido\n";
        let linea = extraer_gc_reply(flujo).expect("debe hallar el reply");
        assert_eq!(linea, "wawactl::gc_reply::vivos=42 muertos=7 sectores=100->58");
    }

    #[test]
    fn gc_reply_incompleto_es_none() {
        // Sin '\n' tras el prefijo aun no hay linea completa: hay que seguir
        // leyendo del transporte.
        assert!(extraer_gc_reply(b"wawactl::gc_reply::vivos=1 muertos=0").is_none());
        assert!(extraer_gc_reply(b"").is_none());
        assert!(extraer_gc_reply(b"trazas sin reply\nmas trazas\n").is_none());
    }

    #[test]
    fn gc_reply_reconoce_error() {
        let flujo = b"wawactl::gc_reply::error::almacen no inicializado\n";
        let linea = extraer_gc_reply(flujo).expect("debe hallar el reply de error");
        assert_eq!(linea, "wawactl::gc_reply::error::almacen no inicializado");
    }
}
