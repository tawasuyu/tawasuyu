//! brainctl: cliente CLI del introspect API.
//!
//! Uso:
//!   cargo run --example brainctl -p ente-brain -- list-rules
//!   cargo run --example brainctl -p ente-brain -- entropy
//!   cargo run --example brainctl -p ente-brain -- top 10
//!   cargo run --example brainctl -p ente-brain -- crystals
//!   cargo run --example brainctl -p ente-brain -- crystal-json 0
//!   cargo run --example brainctl -p ente-brain -- audit 50 --kind kill-ente --kind power-mgmt
//!   cargo run --example brainctl -p ente-brain -- stream-audit --kind kill-ente --since-seq 1000
//!
//! Filtros válidos (--kind): promote-crystal | remove-rule | load-rules-file
//!                           kill-ente | spawn-card-from-disk | brain-inhibit | power-mgmt
//!
//! Path del socket: $ENTE_BRAIN_SOCK o $XDG_RUNTIME_DIR/ente-brain.sock

use arje_brain::introspect::{call, IntrospectRequest, IntrospectResponse};
use arje_brain_audit::audit::{AuditActionKind, AuditFilter};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

fn socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("ENTE_BRAIN_SOCK") {
        return p.into();
    }
    let runtime = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into()));
    format!("{runtime}/ente-brain.sock").into()
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("entropy");

    // Comando especial: streaming. Mantiene la conn abierta y lee frames
    // hasta Ctrl-C o EOF del servidor.
    if cmd == "stream-audit" || cmd == "stream" {
        let filter = parse_filter(&args[2..])?;
        return run_stream_audit(socket_path(), filter).await;
    }

    let req = match cmd {
        "list-rules" | "rules" => IntrospectRequest::ListRules,
        "entropy" => IntrospectRequest::EntropySnapshot,
        "top" => {
            let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
            IntrospectRequest::TopCorrelations { n }
        }
        "crystals" => IntrospectRequest::Crystals,
        "crystal-json" => {
            let i: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            IntrospectRequest::CrystalJson { index: i }
        }
        "promote" => {
            let i: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            IntrospectRequest::PromoteCrystal { index: i }
        }
        "remove" => {
            let id_s = args.get(2).ok_or_else(|| anyhow::anyhow!("se requiere <ulid>"))?;
            let id: ulid::Ulid = id_s.parse()?;
            IntrospectRequest::RemoveRule { id }
        }
        "audit" => {
            // arg 2 puede ser el limit numérico, o el primer flag — si es flag,
            // limit cae a default 20 y todo args[2..] se parsea como filtro.
            let (limit, filter_args) = match args.get(2) {
                Some(s) if s.parse::<usize>().is_ok() => {
                    (s.parse().unwrap(), &args[3..])
                }
                _ => (20usize, &args[2..]),
            };
            let filter = parse_filter(filter_args)?;
            IntrospectRequest::ListAudit { limit, filter }
        }
        "flush-audit" => IntrospectRequest::FlushAudit,
        "audit-verify" | "verify" => IntrospectRequest::VerifyAudit,
        "replay" => IntrospectRequest::ReplayAudit,
        "gc-cas" => IntrospectRequest::GcCas { extra_roots: Vec::new() },
        "patterns" => IntrospectRequest::PatternCrystals,
        "reload" => {
            let path = args.get(2).cloned();
            IntrospectRequest::ReloadRules { path }
        }
        other => {
            eprintln!("subcomando desconocido: {other}");
            eprintln!("válidos: list-rules | entropy | top <n> | crystals | crystal-json <i> | promote <i> | remove <ulid> | audit <limit> | flush-audit | reload [path]");
            std::process::exit(2);
        }
    };

    let path = socket_path();
    let resp = call(&path, req).await?;
    print_response(&resp);
    Ok(())
}

fn print_response(r: &IntrospectResponse) {
    match r {
        IntrospectResponse::Rules(rs) => {
            println!("{} reglas vivas:", rs.len());
            for r in rs {
                println!("  {} prio={} kind={} actions={} wildcard={}",
                    r.id, r.priority, r.event_kind_tag, r.action_count, r.scope_wildcard);
            }
        }
        IntrospectResponse::Rule(rule) => match rule {
            Some(r) => println!("{r:#?}"),
            None => println!("regla no encontrada"),
        },
        IntrospectResponse::Entropy { value_bits, sample_size, distinct_kinds, window_full } => {
            println!("Shannon entropy : {value_bits:.4} bits");
            println!("Sample size     : {sample_size}");
            println!("Distinct kinds  : {distinct_kinds}");
            println!("Window full     : {window_full}");
        }
        IntrospectResponse::Correlations(entries) => {
            println!("{} pares (top, ordenado por co-ocurrencia):", entries.len());
            for e in entries {
                println!("  n={:>4}  P(b|a)={:.3}  PMI={:>6.3}b  {} → {}",
                    e.joint_count, e.conditional_prob, e.pmi_bits, e.a, e.b);
            }
        }
        IntrospectResponse::Crystals(cs) => {
            println!("{} cristales detectados:", cs.len());
            for (i, c) in cs.iter().enumerate() {
                println!("  [{i}] {:?} → {:?}  P={:.3}  PMI={:.3}b  n={}",
                    c.antecedent, c.consequent, c.conditional_prob, c.pmi, c.support);
            }
        }
        IntrospectResponse::Json(s) => println!("{s}"),
        IntrospectResponse::Promoted { rule_id, rule_json } => {
            println!("regla creada: {rule_id}");
            println!("--- JSON para auditoría / persistencia ---");
            println!("{rule_json}");
        }
        IntrospectResponse::Removed(was_present) => {
            if *was_present { println!("regla eliminada"); }
            else { println!("regla no encontrada"); }
        }
        IntrospectResponse::AuditEntries(entries) => {
            println!("{} entries de audit log:", entries.len());
            for e in entries {
                let prev = e.prev_sha.map(hex_short).unwrap_or_else(|| "—".into());
                let sha = hex_short(e.sha);
                println!("  seq={:>4} t={} prev={} sha={}  {:?}",
                    e.seq, e.timestamp_ms, prev, sha, e.action);
            }
        }
        IntrospectResponse::Flushed { written, head_sha, total_flushed } => {
            println!("flushed: {written} entries esta pasada, total acumulado: {total_flushed}");
            if let Some(sha) = head_sha {
                println!("head sha: {}", hex_long(*sha));
            }
        }
        IntrospectResponse::Reloaded { count } => {
            println!("reload OK: {count} reglas activas tras reload");
        }
        IntrospectResponse::Replayed(rep) => {
            if let Some(e) = &rep.error {
                println!("✗ replay falló: {e}");
            } else {
                println!("✓ replay completo — {} actions aplicadas, {} reglas finales",
                    rep.applied, rep.final_rule_count);
            }
        }
        IntrospectResponse::AuditVerified(rep) => {
            if let Some(seq) = rep.broken_at_seq {
                println!("✗ verificación FALLÓ tras seq={seq}");
                if let Some(e) = &rep.error { println!("  motivo: {e}"); }
                println!("  entries verificadas: {}", rep.verified);
            } else {
                println!("✓ chain verificada — {} entries íntegras", rep.verified);
                if let Some(g) = rep.genesis_sha { println!("  genesis: {}", hex_long(g)); }
            }
        }
        IntrospectResponse::Patterns(ps) => {
            println!("{} cristales pattern detectados:", ps.len());
            for p in ps {
                match p {
                    arje_brain::crystallize::PatternCrystal::Burst { kind, count, frequency_per_sec } => {
                        println!("  burst: {kind:?}  count={count}  freq={frequency_per_sec:.2} Hz");
                    }
                    arje_brain::crystallize::PatternCrystal::Silence { kind, last_count, since_secs } => {
                        println!("  silence: {kind:?}  last_count={last_count}  ausente={since_secs:.1}s");
                    }
                }
            }
        }
        IntrospectResponse::GcResult { deleted, freed_bytes } => {
            println!("CAS gc: {deleted} blobs eliminados, {freed_bytes} bytes liberados");
        }
        IntrospectResponse::AuditStreamFrame(_) => {
            // En modo request/response no debería llegar; solo aparece en
            // run_stream_audit. Si llega aquí es un bug del servidor.
            eprintln!("frame de stream recibido fuera de stream-audit (bug)");
        }
        IntrospectResponse::Error(e) => eprintln!("error: {e}"),
    }
}

fn hex_short(sha: [u8; 32]) -> String {
    sha[..4].iter().map(|b| format!("{:02x}", b)).collect::<String>() + ".."
}

fn hex_long(sha: [u8; 32]) -> String {
    sha.iter().map(|b| format!("{:02x}", b)).collect()
}

async fn run_stream_audit(path: PathBuf, filter: AuditFilter) -> anyhow::Result<()> {
    let mut stream = UnixStream::connect(&path).await?;
    let req = IntrospectRequest::StreamAudit { filter };
    let buf = bincode::serialize(&req)?;
    stream.write_u32(buf.len() as u32).await?;
    stream.write_all(&buf).await?;
    eprintln!("audit stream conectado a {} — Ctrl-C para salir", path.display());

    loop {
        let mut len_buf = [0u8; 4];
        if stream.read_exact(&mut len_buf).await.is_err() {
            eprintln!("\nstream cerrado por el servidor");
            return Ok(());
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        if len > 4 * 1024 * 1024 { anyhow::bail!("frame oversize"); }
        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf).await?;
        let resp: IntrospectResponse = bincode::deserialize(&buf)?;
        match resp {
            IntrospectResponse::AuditStreamFrame(entry) => {
                let prev = entry.prev_sha
                    .map(|s| s[..4].iter().map(|b| format!("{:02x}", b)).collect::<String>() + "..")
                    .unwrap_or_else(|| "—".into());
                let sha = entry.sha[..4].iter().map(|b| format!("{:02x}", b))
                    .collect::<String>() + "..";
                println!("[stream] seq={} prev={} sha={}  {:?}",
                    entry.seq, prev, sha, entry.action);
            }
            other => {
                eprintln!("frame no esperado en stream: {other:?}");
                return Ok(());
            }
        }
    }
}

/// Parsea pares `--kind <tag>` (repetible) y `--since-seq <N>` en orden libre.
/// Falla con error claro si una flag aparece sin valor o el tag no se reconoce —
/// preferible a un filtro silenciosamente vacío.
fn parse_filter(args: &[String]) -> anyhow::Result<AuditFilter> {
    let mut filter = AuditFilter::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--kind" => {
                let v = args.get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("--kind requiere un tag"))?;
                let kind = AuditActionKind::parse(v)
                    .ok_or_else(|| anyhow::anyhow!("--kind desconocido: {v}"))?;
                filter.kinds.push(kind);
                i += 2;
            }
            "--since-seq" => {
                let v = args.get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("--since-seq requiere un número"))?;
                filter.since_seq = Some(v.parse()
                    .map_err(|e| anyhow::anyhow!("--since-seq inválido: {e}"))?);
                i += 2;
            }
            other => anyhow::bail!("flag desconocido: {other}"),
        }
    }
    Ok(filter)
}

#[allow(dead_code)]
fn _suppress(_: &Path) {} // mantener Path import si compilador se queja
