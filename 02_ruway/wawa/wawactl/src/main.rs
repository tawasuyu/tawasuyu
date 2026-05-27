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
