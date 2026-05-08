//! Binario `minga`: argument parsing y formateo de salida.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use minga_cli::{
    cmd_ingest, cmd_init, cmd_listen, cmd_status, cmd_sync, cmd_watch, CliError,
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
            println!("MST: {} claves", s.mst_len);
            println!("Nodos almacenados: {}", s.nodes_len);
            println!("Atestaciones: {}", s.attestations_len);
        }
        Command::Ingest { file } => {
            let pass = prompt_passphrase()?;
            let r = cmd_ingest(&cli.repo, &pass, &file)?;
            println!("Ingerido: {}", file.display());
            println!("Hash: {}", r.hash);
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
    }
    Ok(())
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
