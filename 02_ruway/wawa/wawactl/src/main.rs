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
    DaemonFirma {
        /// Path al PTY/dispositivo serial que QEMU expone como COM1.
        #[arg(long)]
        pty: String,
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
        Cmd::DaemonFirma {
            pty,
            clave_privada,
            log,
            auto_firmar_durante,
            slot,
        } => cmd_daemon_firma(
            &pty,
            &clave_privada,
            &log,
            auto_firmar_durante.as_deref(),
            slot,
        ),
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

/// Prefijo de control que el kernel de Wawa emite antes del hash. El parser
/// es estricto: solo lineas que arrancan EXACTAMENTE asi son candidatas.
const PREFIJO_SOLICITUD: &str = "wawa::sign_request::";
/// Longitud del hash en caracteres hexadecimales (32 bytes -> 64 chars).
const HASH_HEX_LEN: usize = 64;
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

/// FASE 39/41 :: subcomando `daemon-firma`. Crea un runtime tokio y delega
/// en `ejecutar_daemon`. Devolver `Err` aqui imprime al stderr de wawactl
/// y sale con codigo no-cero.
fn cmd_daemon_firma(
    pty: &str,
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
    rt.block_on(ejecutar_daemon(
        pty.to_string(),
        sk,
        log_path.to_string(),
        ventana_hasta,
        slot,
    ))
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

        // El parser es ESTRICTO: la linea debe arrancar con el prefijo
        // exacto y traer 64 chars hex justo despues. Cualquier otra
        // basura (logs del kernel, trazas de boot) cae al sumidero
        // silencioso — el operador la ve en su terminal de QEMU igual.
        let trim = linea.trim_end_matches(|c| c == '\n' || c == '\r');
        let Some(hash_hex) = trim.strip_prefix(PREFIJO_SOLICITUD) else {
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
            confirmar_con_operador(hash_hex).await
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
                escribir_auditoria(&log_path, marcador, slot, hash_hex);
                eprintln!(
                    "wawactl: {} slot={} ({} bytes incluyendo prefijo)",
                    marcador,
                    slot,
                    frame.len()
                );
            }
            DecisionOperador::Rechazada => {
                escribir_auditoria(&log_path, "FIRMA_RECHAZADA", slot, hash_hex);
                eprintln!("wawactl: solicitud rechazada o timeout");
            }
        }
    }
}

/// Muestra el prompt al operador y lee `y`/`N` por stdin con timeout. La
/// lectura de stdin es bloqueante (no hay un read async robusto para
/// stdin en tokio sin features adicionales); usamos `spawn_blocking`
/// para que el lazo principal pueda imponer el timeout via
/// `tokio::time::timeout`.
async fn confirmar_con_operador(hash_hex: &str) -> DecisionOperador {
    use std::io::Write;
    eprintln!();
    eprintln!("================================================================");
    eprintln!("  SOLICITUD DE FIRMA DE CUADERNO");
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
/// FIRMA_AUTO_EMITIDA / FIRMA_RECHAZADA), slot del anillo AGORA_AUTH_RING
/// con que se firmo, y el hash. Errores de I/O se imprimen a stderr
/// pero NO interrumpen el lazo — perder una linea de log es preferible
/// a perder el demonio entero. FASE 42 :: el campo `SLOT` distingue las
/// firmas emitidas por distintos dispositivos del operador (primario,
/// secundario, recuperacion).
fn escribir_auditoria(log_path: &str, accion: &str, slot: u8, hash_hex: &str) {
    use std::io::Write;
    let ts = chrono::Utc::now().to_rfc3339();
    let linea = format!(
        "[{ts}] | ACCION: {accion} | SLOT: {slot} | HASH: {hash_hex} | AUTOR: AGORA_AUTH_RING[{slot}]\n"
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
