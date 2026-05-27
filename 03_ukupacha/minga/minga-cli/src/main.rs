//! Binario `minga`: argument parsing y formateo de salida.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use minga_cli::{
    cmd_ingest, cmd_init, cmd_listen, cmd_log, cmd_mount, cmd_show, cmd_status, cmd_sync,
    cmd_watch, CliError,
};

#[derive(Parser)]
#[command(
    name = "minga",
    version,
    about = "Minga: VCS semántico P2P. Versiona AST, no líneas."
)]
struct Cli {
    /// Ruta del repositorio Minga. Por defecto: `.minga` en el cwd.
    #[arg(short, long, default_value = ".minga", global = true)]
    repo: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inicializa un nuevo repo: genera keypair Ed25519, lo cifra con
    /// passphrase, y crea el almacén persistente vacío.
    Init,

    /// Muestra DID, tamaño del MST, nodos almacenados y atestaciones.
    Status,

    /// Parsea un archivo Rust, lo añade al MST y firma una atestación
    /// de autoría con la identidad del repo.
    Ingest {
        /// Ruta del archivo .rs a ingerir.
        file: PathBuf,
    },

    /// Escucha conexiones de peers en una multiaddr libp2p y acepta
    /// sincronizaciones entrantes hasta Ctrl+C.
    Listen {
        /// Multiaddr libp2p, ej. `/ip4/0.0.0.0/tcp/4001`.
        addr: String,
    },

    /// Sincroniza una vez con un peer remoto (multiaddr con `/p2p/<id>`).
    Sync {
        /// Multiaddr completo, ej. `/ip4/1.2.3.4/tcp/4001/p2p/12D3KooW...`.
        peer: String,
    },

    /// Vigila un directorio y re-ingiere automáticamente cualquier
    /// archivo `.rs` que se cree o modifique. Minga como VCS de fondo:
    /// el usuario escribe en su editor y el código queda versionado.
    Watch {
        /// Directorio a vigilar.
        dir: PathBuf,
    },

    /// Monta el repositorio como filesystem FUSE de sólo lectura.
    /// Cada hash del store se vuelve un archivo navegable con
    /// `ls`/`cat`. Bloquea hasta `fusermount -u <punto>`.
    Mount {
        /// Punto de montaje: un directorio existente.
        point: PathBuf,
    },

    /// Lista atestaciones del repo ordenadas por timestamp descendente.
    /// Si se pasa un archivo, marca la entrada cuyo α-hash coincide
    /// con el contenido actual del archivo.
    Log {
        /// Archivo cuyo α-hash debe marcarse como "current" (opcional).
        file: Option<PathBuf>,
    },

    /// Pinta el contenido del nodo identificado por `hash`. Acepta
    /// α-hashes (raíces) y hashes estructurales del grafo interno.
    Show {
        /// Hash en hex (64 caracteres).
        hash: String,
        /// Si se pasa, devuelve el árbol como S-expression en vez de
        /// la fuente reconstruida.
        #[arg(long)]
        sexp: bool,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), CliError> {
    let cli = Cli::parse();

    match cli.command {
        Command::Init => {
            let pass = prompt_passphrase_with_confirm()?;
            let did = cmd_init(&cli.repo, &pass)?;
            println!("Repo inicializado en {}", cli.repo.display());
            println!("DID: {}", did);
        }
        Command::Status => {
            let pass = prompt_passphrase()?;
            let s = cmd_status(&cli.repo, &pass)?;
            println!("DID: {}", s.did);
            println!("Raíces (α): {}", s.roots_len);
            println!("MST: {} claves", s.mst_len);
            println!("Nodos almacenados: {}", s.nodes_len);
            println!("Atestaciones: {}", s.attestations_len);
        }
        Command::Ingest { file } => {
            let pass = prompt_passphrase()?;
            let r = cmd_ingest(&cli.repo, &pass, &file)?;
            println!("Ingerido: {} ({})", file.display(), r.dialect.name());
            println!("α-hash: {}", r.alpha);
            println!("struct: {}", r.struct_hash);
            println!("Firmado por: {}", r.did);
        }
        Command::Listen { addr } => {
            let pass = prompt_passphrase()?;
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| CliError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
            rt.block_on(cmd_listen(&cli.repo, &pass, &addr))?;
        }
        Command::Sync { peer } => {
            let pass = prompt_passphrase()?;
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| CliError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
            rt.block_on(cmd_sync(&cli.repo, &pass, &peer))?;
            println!("Sync completo.");
        }
        Command::Watch { dir } => {
            let pass = prompt_passphrase()?;
            println!("Vigilando {}. Ctrl+C para parar.", dir.display());
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| CliError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
            rt.block_on(cmd_watch(&cli.repo, &pass, &dir))?;
        }
        Command::Mount { point } => {
            let pass = prompt_passphrase()?;
            println!(
                "Montando {} en {}. `fusermount -u {}` para desmontar.",
                cli.repo.display(),
                point.display(),
                point.display()
            );
            cmd_mount(&cli.repo, &pass, &point)?;
        }
        Command::Log { file } => {
            let pass = prompt_passphrase()?;
            let entries = cmd_log(&cli.repo, &pass, file.as_deref())?;
            if entries.is_empty() {
                println!("(repo sin atestaciones)");
            }
            for e in entries {
                let mark = if e.current { "*" } else { " " };
                let when = format_ts(e.ts_secs);
                let dialect = e.dialect.map(|d| d.name()).unwrap_or("?");
                println!("{} {}  {}  [{}]  by {}", mark, when, e.alpha, dialect, e.author);
            }
        }
        Command::Show { hash, sexp } => {
            let pass = prompt_passphrase()?;
            let r = cmd_show(&cli.repo, &pass, &hash, sexp)?;
            if r.is_root {
                let dialect = r.dialect.map(|d| d.name()).unwrap_or("?");
                eprintln!(
                    "# raíz α={} → struct={} ({})",
                    r.alpha.unwrap(),
                    r.struct_hash,
                    dialect
                );
            } else {
                eprintln!("# nodo estructural {}", r.struct_hash);
            }
            print!("{}", r.rendered);
        }
    }
    Ok(())
}

/// Formato compacto del timestamp Unix → `YYYY-MM-DD HH:MM` UTC.
fn format_ts(secs: u64) -> String {
    if secs == 0 {
        return "    (sin fecha)   ".to_string();
    }
    // Convertir segundos Unix a una fecha legible UTC sin chrono.
    // Algoritmo: días civiles desde epoch + descomposición Howard Hinnant.
    let days = (secs / 86_400) as i64;
    let secs_of_day = secs % 86_400;
    let h = secs_of_day / 3600;
    let m = (secs_of_day % 3600) / 60;
    let (y, mo, d) = civil_from_days(days + 719_468);
    format!("{:04}-{:02}-{:02} {:02}:{:02}", y, mo, d, h, m)
}

/// Howard Hinnant — días desde Mar 1, 0000 (sistema proléptico) a (Y, M, D).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn prompt_passphrase() -> Result<String, CliError> {
    let pass = rpassword::prompt_password("Passphrase: ")
        .map_err(CliError::Io)?;
    Ok(pass)
}

fn prompt_passphrase_with_confirm() -> Result<String, CliError> {
    let pass = rpassword::prompt_password("Passphrase nueva: ")
        .map_err(CliError::Io)?;
    let conf = rpassword::prompt_password("Confirma: ")
        .map_err(CliError::Io)?;
    if pass != conf {
        return Err(CliError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "passphrases no coinciden",
        )));
    }
    Ok(pass)
}
