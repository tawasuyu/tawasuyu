//! Binario `minga`: argument parsing y formateo de salida.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use minga_cli::{
    cmd_blame, cmd_bundle_export, cmd_bundle_export_all, cmd_bundle_import, cmd_bundle_import_all,
    cmd_diff, cmd_history, cmd_ingest, cmd_ingest_dir, cmd_init, cmd_listen, cmd_log, cmd_mount,
    cmd_prune, cmd_retire, cmd_roots, cmd_serve, cmd_show, cmd_sign, cmd_signers, cmd_status,
    cmd_sync, cmd_verify_root, cmd_watch, CliError, DiffLine,
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

    /// Ingiere todos los archivos soportados de un directorio en una
    /// sola pasada (sin dejar `watch` corriendo). Útil para versionar un
    /// repo entero on-demand.
    IngestDir {
        /// Directorio a recorrer.
        dir: PathBuf,
        /// Descender recursivamente. Los dot-dirs (`.git`, `.minga`, …)
        /// se saltan durante el descenso para evitar ruido.
        #[arg(short, long)]
        recursive: bool,
    },

    /// Firma una atestación bajo el keypair local sobre un α-hash
    /// existente — vouching colaborativo. A diferencia de `ingest`, no
    /// crea contenido nuevo: sólo declara aval sobre algo ya en el repo
    /// (típicamente traído por `sync` desde otro peer).
    Sign {
        /// α-hash en hex (64 caracteres).
        hash: String,
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
        /// Compara contra otro hash (atajo de `minga diff`). Mutuamente
        /// excluyente con `--sexp`.
        #[arg(long = "diff-against")]
        diff_against: Option<String>,
    },

    /// Compara dos versiones del repo (típicamente dos α-hashes) y
    /// muestra el diff unified de sus `render_source`.
    Diff {
        /// Hash izquierdo en hex.
        left: String,
        /// Hash derecho en hex.
        right: String,
    },

    /// Retira una raíz: emite una atestación negativa firmada por el
    /// keypair del repo y quita el α-hash del MST/`roots`. Las
    /// atestaciones originales se conservan como prueba histórica.
    Retire {
        /// α-hash en hex (64 caracteres).
        hash: String,
    },

    /// Verifica que el α-hash de una raíz local es consistente con su
    /// contenido bajo algún dialecto soportado. Útil para auditar
    /// raíces traídas por sync (cuyo dialect no necesariamente está
    /// registrado, o cuyo remitente puede no ser confiable).
    Verify {
        /// α-hash en hex (64 caracteres).
        hash: String,
    },

    /// Recolector de basura del grafo CAS: borra nodos no alcanzables
    /// desde ninguna raíz (típicamente quedan tras `retire`/`watch`
    /// Remove). Idempotente.
    Prune,

    /// Para cada línea del archivo registrado, muestra el α-hash que la
    /// introdujo. Reconstruye la cadena de versiones del path desde su
    /// historial (poblado por `ingest`/`watch`) y propaga la atribución
    /// hacia adelante con diffs línea-a-línea.
    Blame {
        /// Archivo cuyo historial atribuir.
        file: PathBuf,
    },

    /// Lista todas las raíces del repo con su path conocido, dialect,
    /// fecha de última atestación y cantidad de firmas. Ordenado por
    /// actividad reciente — útil cuando no recordás un α-hash.
    Roots,

    /// Historial cronológico (más reciente primero) de un path: cada
    /// α-hash que pasó por esa ruta vía `ingest`/`watch`. Marca con `*`
    /// la entrada cuyo α coincide con el contenido actual del archivo.
    History {
        /// Archivo cuyo historial mostrar.
        file: PathBuf,
    },

    /// Lista los DIDs que han atestado un α-hash, con timestamp local
    /// de cuándo se observó la firma. Marca quienes también firmaron
    /// una retracción posterior.
    Signers {
        /// α-hash en hex (64 caracteres).
        hash: String,
        /// Filtra firmas observadas desde ese instante. Acepta:
        /// `YYYY-MM-DD` (medianoche UTC), o un relativo con sufijo
        /// `m`/`h`/`d`/`w` (`30d`, `12h`, `2w`). Atestaciones sin
        /// timestamp local (legacy) quedan fuera del filtro.
        #[arg(long)]
        since: Option<String>,
    },

    /// Bundle: empaquetar / desempaquetar una raíz para transferencia
    /// offline (USB-stick) — mismo nivel de verificación criptográfica
    /// que el wire libp2p, sin necesidad de red.
    #[command(subcommand)]
    Bundle(BundleCommand),

    /// Levanta un daemon HTTP read-only sobre el repo. Útil para
    /// integrar minga con frontends no-Rust (web, mobile, otro shell).
    /// El passphrase se mantiene en memoria mientras el daemon corre.
    Serve {
        /// Socket de escucha (ej. `127.0.0.1:7777`).
        addr: String,
        /// Si se pasa, exige `Authorization: Bearer <token>` en cada
        /// request — sin token, el daemon corre abierto (ok sólo en
        /// `127.0.0.1`). Para no exponer el token en el `ps`,
        /// también se acepta vía env `MINGA_SERVE_TOKEN`.
        #[arg(long)]
        token: Option<String>,
    },
}

#[derive(Subcommand)]
enum BundleCommand {
    /// Empaqueta α-hash + nodos alcanzables + atestaciones + retracciones
    /// en un archivo postcard portable.
    Export {
        /// α-hash en hex (64 caracteres).
        hash: String,
        /// Ruta de salida del bundle (sobreescribe si existe).
        out: PathBuf,
    },

    /// Empaqueta TODAS las raíces del repo en un solo multi-bundle.
    /// Las raíces sin dialect persistido (sync'd bajo wire pre-RootDeclaration)
    /// se saltan; se reportan al final.
    ExportAll {
        /// Ruta de salida del multi-bundle (sobreescribe si existe).
        out: PathBuf,
    },

    /// Lee un bundle single, re-verifica criptográficamente cada pieza,
    /// y mergea idempotentemente en los stores locales.
    Import {
        /// Ruta del bundle a importar.
        file: PathBuf,
    },

    /// Lee un multi-bundle y mergea cada raíz contenida con las mismas
    /// garantías que `import`. Idempotente.
    ImportAll {
        /// Ruta del multi-bundle a importar.
        file: PathBuf,
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
        Command::IngestDir { dir, recursive } => {
            let pass = prompt_passphrase()?;
            let s = cmd_ingest_dir(&cli.repo, &pass, &dir, recursive)?;
            println!(
                "Bulk ingest en {}: {} archivos vistos, {} ingeridos, {} fallos",
                dir.display(),
                s.seen,
                s.ingested,
                s.failed.len()
            );
            for (p, msg) in s.failed {
                eprintln!("  ✘ {}: {}", p.display(), msg);
            }
        }
        Command::Sign { hash } => {
            let pass = prompt_passphrase()?;
            let r = cmd_sign(&cli.repo, &pass, &hash)?;
            if r.is_new_attestation {
                println!("Firmado α-hash {} por {}", r.alpha, r.author);
            } else {
                println!(
                    "Atestación ya existente para {} bajo {} (idempotente, no se duplica)",
                    r.alpha, r.author
                );
            }
            if !r.is_known_root {
                println!(
                    "⚠ aviso: {} no está registrado como raíz local (firmaste igual)",
                    r.alpha
                );
            }
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
        Command::Prune => {
            let pass = prompt_passphrase()?;
            let s = cmd_prune(&cli.repo, &pass)?;
            println!(
                "Prune: {} raíces · {} nodos antes · {} alcanzables · {} borrados",
                s.roots, s.before, s.alive, s.removed
            );
        }
        Command::Verify { hash } => {
            let pass = prompt_passphrase()?;
            let v = cmd_verify_root(&cli.repo, &pass, &hash)?;
            println!("α-hash:    {}", v.alpha);
            println!("struct:    {}", v.struct_hash);
            println!(
                "registrado: {}",
                v.stored_dialect.map(|d| d.name()).unwrap_or("(huérfano — sin entrada en `roots`)")
            );
            match v.verified_dialect {
                Some(d) => println!("verificado: OK como {}", d.name()),
                None => {
                    println!("verificado: ✘ INCONSISTENTE — ningún dialecto produce ese α-hash");
                    std::process::exit(2);
                }
            }
            if !v.matches_stored() && v.stored_dialect.is_some() {
                println!("⚠ aviso: dialect registrado ≠ verificado (posible drift)");
            }
        }
        Command::Retire { hash } => {
            let pass = prompt_passphrase()?;
            let r = cmd_retire(&cli.repo, &pass, &hash)?;
            if r.was_root {
                println!("Retirada raíz {}", r.alpha);
            } else {
                println!(
                    "Atestación negativa firmada para {} (no era raíz local)",
                    r.alpha
                );
            }
            println!("Firmada por: {}", r.author);
        }
        Command::Diff { left, right } => {
            let pass = prompt_passphrase()?;
            let d = cmd_diff(&cli.repo, &pass, &left, &right)?;
            let left_kind = if d.left_is_root { "α" } else { "struct" };
            let right_kind = if d.right_is_root { "α" } else { "struct" };
            eprintln!("--- {} ({})", d.left_hash, left_kind);
            eprintln!("+++ {} ({})", d.right_hash, right_kind);
            eprintln!(
                "@@ +{} −{} @@",
                d.additions, d.deletions
            );
            for line in d.lines {
                match line {
                    DiffLine::Same(t) => print!(" {t}"),
                    DiffLine::Add(t) => print!("+{t}"),
                    DiffLine::Remove(t) => print!("-{t}"),
                }
            }
        }
        Command::Show {
            hash,
            sexp,
            diff_against,
        } => {
            let pass = prompt_passphrase()?;
            if let Some(other) = diff_against {
                // Atajo: show --diff-against <other> ≡ minga diff <hash> <other>.
                if sexp {
                    eprintln!("--sexp y --diff-against son mutuamente excluyentes");
                    return Err(CliError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "flags incompatibles",
                    )));
                }
                let d = cmd_diff(&cli.repo, &pass, &hash, &other)?;
                eprintln!("--- {}", d.left_hash);
                eprintln!("+++ {}", d.right_hash);
                eprintln!("@@ +{} −{} @@", d.additions, d.deletions);
                for line in d.lines {
                    match line {
                        DiffLine::Same(t) => print!(" {t}"),
                        DiffLine::Add(t) => print!("+{t}"),
                        DiffLine::Remove(t) => print!("-{t}"),
                    }
                }
            } else {
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
        Command::Blame { file } => {
            let pass = prompt_passphrase()?;
            let lines = cmd_blame(&cli.repo, &pass, &file)?;
            for line in lines {
                let short: String = line.alpha.to_string().chars().take(12).collect();
                let when = format_ts(line.ts_secs);
                println!("{} {} {} | {}", short, when, line.author, line.text);
            }
        }
        Command::Roots => {
            let pass = prompt_passphrase()?;
            let rows = cmd_roots(&cli.repo, &pass)?;
            if rows.is_empty() {
                println!("(repo sin raíces)");
            }
            for r in rows {
                let when = format_ts(r.last_seen_secs);
                let dialect = r.dialect.map(|d| d.name()).unwrap_or("?");
                let short: String = r.alpha.to_string().chars().take(12).collect();
                let path = r.path.as_deref().unwrap_or("(sin path local)");
                println!(
                    "{}  {}  [{:<6}]  ×{}  {}",
                    short, when, dialect, r.attestations, path
                );
            }
        }
        Command::History { file } => {
            let pass = prompt_passphrase()?;
            let entries = cmd_history(&cli.repo, &pass, &file)?;
            for e in entries {
                let mark = if e.current { "*" } else { " " };
                let when = format_ts(e.ts_secs);
                let dialect = e.dialect.map(|d| d.name()).unwrap_or("?");
                println!("{} {}  {}  [{}]", mark, when, e.alpha, dialect);
            }
        }
        Command::Signers { hash, since } => {
            let pass = prompt_passphrase()?;
            let since_secs = match since.as_deref() {
                Some(s) => Some(parse_since(s)?),
                None => None,
            };
            let entries = cmd_signers(&cli.repo, &pass, &hash, since_secs)?;
            if entries.is_empty() {
                let extra = match since_secs {
                    Some(_) => " bajo el filtro --since",
                    None => "",
                };
                println!("(sin atestaciones locales para ese α-hash{extra})");
            }
            for e in entries {
                let when = format_ts(e.ts_secs);
                let marker = if e.retracted { "↺" } else { " " };
                println!("{} {}  {}", marker, when, e.author);
            }
        }
        Command::Bundle(BundleCommand::Export { hash, out }) => {
            let pass = prompt_passphrase()?;
            let s = cmd_bundle_export(&cli.repo, &pass, &hash, &out)?;
            println!("Bundle escrito en {}", out.display());
            println!("  α-hash:        {}", s.alpha);
            println!("  nodos:         {}", s.nodes);
            println!("  atestaciones:  {}", s.attestations);
            println!("  retractions:   {}", s.retractions);
            println!("  tamaño:        {} bytes", s.bytes);
        }
        Command::Bundle(BundleCommand::Import { file }) => {
            let pass = prompt_passphrase()?;
            let s = cmd_bundle_import(&cli.repo, &pass, &file)?;
            println!("Bundle importado: α-hash {}", s.alpha);
            if s.root_was_new {
                println!("  raíz nueva, registrada en MST y `roots`");
            } else {
                println!("  raíz ya conocida (idempotente)");
            }
            println!("  nodos insertados:        {}", s.nodes_inserted);
            println!(
                "  atestaciones:            {} nuevas, {} rechazadas",
                s.attestations_added, s.attestations_rejected
            );
            println!(
                "  retractions:             {} nuevas, {} rechazadas",
                s.retractions_added, s.retractions_rejected
            );
        }
        Command::Bundle(BundleCommand::ExportAll { out }) => {
            let pass = prompt_passphrase()?;
            let s = cmd_bundle_export_all(&cli.repo, &pass, &out)?;
            println!("Multi-bundle escrito en {}", out.display());
            println!("  raíces:        {}", s.roots);
            println!("  nodos:         {}", s.total_nodes);
            println!("  atestaciones:  {}", s.total_attestations);
            println!("  retractions:   {}", s.total_retractions);
            println!(
                "  tamaño:        {} bytes (zstd · raw {} bytes, ratio {:.2}×)",
                s.bytes,
                s.uncompressed_bytes,
                if s.bytes == 0 {
                    0.0
                } else {
                    s.uncompressed_bytes as f64 / s.bytes as f64
                }
            );
            if !s.skipped_missing_dialect.is_empty() {
                println!(
                    "  ⚠ {} raíces saltadas (sin dialect registrado):",
                    s.skipped_missing_dialect.len()
                );
                for h in s.skipped_missing_dialect {
                    println!("      {}", h);
                }
            }
        }
        Command::Serve { addr, token } => {
            let pass = prompt_passphrase()?;
            // Fallback al env si --token no se pasó. La env es el camino
            // recomendado para evitar exponer el secreto en el `ps`.
            let env_tok = std::env::var("MINGA_SERVE_TOKEN").ok();
            let token = token.or(env_tok);
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| CliError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
            rt.block_on(cmd_serve(&cli.repo, &pass, &addr, token.as_deref()))?;
        }
        Command::Bundle(BundleCommand::ImportAll { file }) => {
            let pass = prompt_passphrase()?;
            let s = cmd_bundle_import_all(&cli.repo, &pass, &file)?;
            println!(
                "Multi-bundle importado: {} raíces ({} nuevas)",
                s.items.len(),
                s.roots_new()
            );
            println!("  nodos insertados:        {}", s.total_nodes_inserted());
            println!(
                "  atestaciones nuevas:     {}",
                s.total_attestations_added()
            );
            println!(
                "  retractions nuevas:      {}",
                s.total_retractions_added()
            );
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

/// Resuelve `--since` a un instante Unix (UTC). Acepta:
/// - `YYYY-MM-DD` → medianoche UTC de ese día;
/// - sufijo de duración `Nm` / `Nh` / `Nd` / `Nw` → `now - N`.
///
/// El parser es estricto para no confundir un typo con una fecha
/// silenciosamente válida; cualquier formato inesperado produce
/// `InvalidInput` con un mensaje claro.
fn parse_since(s: &str) -> Result<u64, CliError> {
    let s = s.trim();
    // Forma absoluta: YYYY-MM-DD.
    if let Some((y, rest)) = s.split_once('-') {
        if let Some((mo, d)) = rest.split_once('-') {
            let y: i64 = y.parse().map_err(|_| since_err(s))?;
            let mo: u32 = mo.parse().map_err(|_| since_err(s))?;
            let d: u32 = d.parse().map_err(|_| since_err(s))?;
            if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
                return Err(since_err(s));
            }
            let days = days_from_civil(y, mo, d);
            let unix_days = days - 719_468;
            if unix_days < 0 {
                return Err(since_err(s));
            }
            return Ok((unix_days as u64) * 86_400);
        }
    }
    // Forma relativa: digit+ + sufijo.
    let last = s.chars().last().ok_or_else(|| since_err(s))?;
    let (num, mult) = match last {
        'm' => (&s[..s.len() - 1], 60u64),
        'h' => (&s[..s.len() - 1], 3_600u64),
        'd' => (&s[..s.len() - 1], 86_400u64),
        'w' => (&s[..s.len() - 1], 7 * 86_400u64),
        _ => return Err(since_err(s)),
    };
    let n: u64 = num.parse().map_err(|_| since_err(s))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok(now.saturating_sub(n.saturating_mul(mult)))
}

fn since_err(input: &str) -> CliError {
    CliError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!(
            "--since '{input}' inválido: usá YYYY-MM-DD o un relativo \
             como 30d / 12h / 2w / 15m"
        ),
    ))
}

/// Inversa de `civil_from_days`: cuenta de días absolutos según el
/// algoritmo de Howard Hinnant. Resultado se interpreta a Unix days
/// restándole 719_468.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let m = m as u64;
    let d = d as u64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64
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
