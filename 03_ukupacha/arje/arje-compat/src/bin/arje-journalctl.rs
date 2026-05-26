//! ente-journalctl: query CLI sobre el journal persistido en CAS.
//!
//! Lee el index `~/.local/share/ente/journal/index.log` (líneas
//! `timestamp_ms:source:unit:sha_hex`), filtra, y para cada match
//! restituye el blob desde CAS y lo imprime.
//!
//! Uso:
//!   ente-journalctl                       # todo el journal
//!   ente-journalctl --unit foo.service    # filtra por unit
//!   ente-journalctl --since 60            # últimos 60 segundos
//!   ente-journalctl --grep "panic"        # contiene "panic"
//!   ente-journalctl --tail 20             # últimas 20 entries
//!   ente-journalctl --json                # output JSON-lines

use std::path::PathBuf;

struct Args {
    unit: Option<String>,
    since_secs: Option<u64>,
    grep: Option<String>,
    tail: Option<usize>,
    source: Option<String>,
    output: OutputFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Pretty,
    Json,
    /// systemd journal export format: `KEY=value\n` por field, blank line
    /// entre entries. Documented at https://systemd.io/JOURNAL_EXPORT_FORMATS/
    /// Compatible con `journalctl --input-format=export`.
    Export,
}

fn parse_args() -> Args {
    let mut args = std::env::args().skip(1);
    let mut a = Args {
        unit: None, since_secs: None, grep: None, tail: None,
        source: None, output: OutputFormat::Pretty,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--unit" | "-u" => a.unit = args.next(),
            "--since" | "-S" => a.since_secs = args.next().and_then(|s| s.parse().ok()),
            "--grep" | "-g" => a.grep = args.next(),
            "--tail" | "-n" => a.tail = args.next().and_then(|s| s.parse().ok()),
            "--source" => a.source = args.next(),
            "--json" => a.output = OutputFormat::Json,
            "--output" | "-o" => {
                a.output = match args.next().as_deref() {
                    Some("pretty") | None => OutputFormat::Pretty,
                    Some("json") | Some("json-lines") => OutputFormat::Json,
                    Some("export") => OutputFormat::Export,
                    Some(other) => {
                        eprintln!("output desconocido: {other}");
                        eprintln!("válidos: pretty | json | export");
                        std::process::exit(2);
                    }
                };
            }
            "-h" | "--help" => { print_help(); std::process::exit(0); }
            other => {
                eprintln!("argumento desconocido: {other}");
                print_help();
                std::process::exit(2);
            }
        }
    }
    a
}

fn print_help() {
    eprintln!("ente-journalctl — query CLI del journal persistido en CAS");
    eprintln!();
    eprintln!("Filtros:");
    eprintln!("  --unit, -u <name>    Filtra por unidad (e.g. foo.service)");
    eprintln!("  --source <s>         journal | syslog");
    eprintln!("  --since, -S <secs>   Sólo últimos N segundos");
    eprintln!("  --grep, -g <text>    Contiene <text> en el body decoded");
    eprintln!("  --tail, -n <N>       Últimas N entries");
    eprintln!("Output:");
    eprintln!("  --output, -o <fmt>   pretty | json | export (systemd journal export)");
    eprintln!("  --json               alias de --output json");
}

fn index_path() -> PathBuf {
    let base = if let Ok(d) = std::env::var("XDG_DATA_HOME") { d }
        else if let Ok(h) = std::env::var("HOME") { format!("{h}/.local/share") }
        else { "/var/lib".into() };
    PathBuf::from(base).join("ente").join("journal").join("index.log")
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[derive(Debug)]
struct IndexEntry {
    timestamp_ms: u128,
    source: String,
    unit: String,
    sha_hex: String,
}

fn parse_line(line: &str) -> Option<IndexEntry> {
    let mut parts = line.splitn(4, ':');
    let ts: u128 = parts.next()?.parse().ok()?;
    let source = parts.next()?.to_string();
    let unit = parts.next()?.to_string();
    let sha = parts.next()?.to_string();
    if sha.len() != 64 { return None; }
    Some(IndexEntry { timestamp_ms: ts, source, unit, sha_hex: sha })
}

fn parse_sha(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 { return None; }
    let mut sha = [0u8; 32];
    for i in 0..32 {
        sha[i] = u8::from_str_radix(&hex[i*2..i*2+2], 16).ok()?;
    }
    Some(sha)
}

fn main() -> anyhow::Result<()> {
    let args = parse_args();
    let path = index_path();
    if !path.exists() {
        eprintln!("index no existe: {} — ¿journald-compat ha corrido?", path.display());
        std::process::exit(1);
    }
    let raw = std::fs::read_to_string(&path)?;
    let mut entries: Vec<IndexEntry> = raw.lines()
        .filter_map(parse_line)
        .collect();

    // Filtros
    let now = now_ms();
    if let Some(secs) = args.since_secs {
        let cutoff = now.saturating_sub(secs as u128 * 1000);
        entries.retain(|e| e.timestamp_ms >= cutoff);
    }
    if let Some(unit) = &args.unit {
        entries.retain(|e| &e.unit == unit);
    }
    if let Some(src) = &args.source {
        entries.retain(|e| &e.source == src);
    }
    // tail después de filtros temporales/identidad pero antes de grep —
    // grep es post porque requiere cargar bytes del CAS.

    let mut out: Vec<(IndexEntry, String)> = entries.into_iter()
        .filter_map(|e| {
            let sha = parse_sha(&e.sha_hex)?;
            let bytes = arje_cas::resolve(&sha).ok()?;
            let body = String::from_utf8_lossy(&bytes).into_owned();
            Some((e, body))
        })
        .collect();

    if let Some(g) = &args.grep {
        out.retain(|(_, body)| body.contains(g.as_str()));
    }
    if let Some(n) = args.tail {
        let len = out.len();
        if len > n { out.drain(..len - n); }
    }

    for (e, body) in out {
        match args.output {
            OutputFormat::Pretty => print_pretty(&e, &body),
            OutputFormat::Json => print_json(&e, &body),
            OutputFormat::Export => print_export(&e, &body),
        }
    }
    Ok(())
}

fn print_pretty(e: &IndexEntry, body: &str) {
    let secs = e.timestamp_ms / 1000;
    let ms = e.timestamp_ms % 1000;
    let header = if e.unit == "-" {
        format!("{}.{:03} [{}]", secs, ms, e.source)
    } else {
        format!("{}.{:03} [{}] {{{}}}", secs, ms, e.source, e.unit)
    };
    println!("{header}");
    // Si es journald native (KEY=value lines), extraer MESSAGE.
    if body.contains('=') && body.lines().any(|l| l.contains('=')) {
        for line in body.lines() {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == "MESSAGE" {
                    println!("  {v}");
                    return;
                }
            }
        }
    }
    for line in body.trim_end().lines() {
        println!("  {line}");
    }
}

/// systemd journal export format. Cada entry es un bloque de líneas
/// `KEY=value\n` separado por blank line. Para values con newlines o
/// bytes binarios, el format usa una variante con length-prefix
/// (8 bytes LE u64) — por simplicidad sólo emitimos values con texto
/// que no contienen newlines o caracteres no-printables. Extraemos
/// MESSAGE/PRIORITY/_SYSTEMD_UNIT del body si es journald native.
///
/// Compatible con `journalctl --input-format=export -m`.
fn print_export(e: &IndexEntry, body: &str) {
    // Timestamps: __REALTIME_TIMESTAMP en µs, __MONOTONIC_TIMESTAMP también.
    let realtime_us = e.timestamp_ms.saturating_mul(1000);
    println!("__CURSOR=s={};t={};x={}",
        &e.sha_hex[..16],     // pseudo-cursor: prefix del SHA
        realtime_us,
        &e.sha_hex[..8]);
    println!("__REALTIME_TIMESTAMP={}", realtime_us);
    println!("__MONOTONIC_TIMESTAMP={}", realtime_us);

    let host = gethostname_safe();
    if !host.is_empty() {
        println!("_HOSTNAME={host}");
    }

    if e.unit != "-" {
        println!("_SYSTEMD_UNIT={}", e.unit);
    }
    println!("_TRANSPORT={}", match e.source.as_str() {
        "syslog" => "syslog",
        "journal" => "journal",
        _ => "stdout",
    });

    // Si el body es journald native (KEY=value lines), emitir cada uno
    // verbatim — son los fields originales del producer. Filtrar líneas
    // que no son seguras para export (con newlines en value, etc).
    if body.contains('=') && body.lines().any(|l| l.contains('=')) {
        for line in body.lines() {
            if line.contains('=') && line.bytes().all(safe_export_byte) {
                println!("{line}");
            }
        }
    } else {
        // Syslog text — empaquetar como MESSAGE.
        let msg = body.trim_end()
            .replace('\n', " ");  // collapsa newlines
        if msg.bytes().all(safe_export_byte) {
            println!("MESSAGE={msg}");
        }
    }
    // Blank line separa entries.
    println!();
}

fn safe_export_byte(b: u8) -> bool {
    // ASCII printable, espacio, tab. No newlines (manejados aparte).
    (0x20..=0x7E).contains(&b) || b == b'\t'
}

fn gethostname_safe() -> String {
    let mut buf = [0u8; 256];
    let r = unsafe {
        libc::gethostname(buf.as_mut_ptr() as *mut _, buf.len())
    };
    if r != 0 { return String::new(); }
    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    std::str::from_utf8(&buf[..len]).unwrap_or("").to_string()
}

fn print_json(e: &IndexEntry, body: &str) {
    // JSON-lines básico, sin dependencia de serde — format simple y estable.
    let escaped_body = body
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    let unit_field = if e.unit == "-" { "null".to_string() }
        else { format!("\"{}\"", e.unit) };
    println!(
        r#"{{"timestamp_ms":{},"source":"{}","unit":{},"sha":"{}","body":"{}"}}"#,
        e.timestamp_ms, e.source, unit_field, e.sha_hex, escaped_body
    );
}
