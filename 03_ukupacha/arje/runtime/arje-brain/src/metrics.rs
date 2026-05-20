//! Endpoint Prometheus en TCP. Formato text/plain (exposition format 0.0.4).
//!
//! Sin dependencias adicionales — la cardinalidad de nuestras métricas es
//! pequeña y el formato es trivial. Si crece, sustituir por la crate
//! `prometheus` con su Registry + encoders.

use crate::introspect::BrainState;
use arje_brain_rules::rules::EventKind;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{info, trace, warn};

/// Lanza el listener Prometheus. Devuelve cuando bind() falla; en caso
/// contrario corre indefinidamente. Pensado para `tokio::spawn`.
pub async fn serve_metrics(state: BrainState, addr: SocketAddr) -> anyhow::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    info!(?addr, "prometheus /metrics escuchando");
    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                trace!(?peer, "metrics scrape");
                let s = state.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_scrape(stream, s).await {
                        warn!(?e, "metrics conn ended");
                    }
                });
            }
            Err(e) => {
                warn!(?e, "metrics accept failed");
                return Ok(());
            }
        }
    }
}

async fn handle_scrape(mut stream: TcpStream, state: BrainState) -> anyhow::Result<()> {
    // Drenamos el request line + headers sin parsear (cualquier path
    // responde igual — Prometheus envía GET /metrics típicamente).
    let mut buf = [0u8; 1024];
    let _ = stream.read(&mut buf).await;
    let body = format_metrics(&state).await;
    let resp = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/plain; version=0.0.4\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        body.len(), body
    );
    stream.write_all(resp.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

async fn format_metrics(state: &BrainState) -> String {
    let obs = state.observer.read().await;
    let engine = state.engine.read().await;
    let audit = state.audit.read().await;

    let mut out = String::with_capacity(2048);

    // ---- Entropía ----
    out.push_str("# HELP ente_brain_entropy_bits Shannon entropy of marginal event distribution.\n");
    out.push_str("# TYPE ente_brain_entropy_bits gauge\n");
    out.push_str(&format!("ente_brain_entropy_bits {:.6}\n", obs.shannon_entropy()));

    // ---- Tamaño de muestra ----
    out.push_str("# HELP ente_brain_events_total Total events recorded by the observer.\n");
    out.push_str("# TYPE ente_brain_events_total counter\n");
    out.push_str(&format!("ente_brain_events_total {}\n", obs.total()));

    // ---- Distinct kinds ----
    out.push_str("# HELP ente_brain_distinct_kinds Number of distinct EventKind tags seen.\n");
    out.push_str("# TYPE ente_brain_distinct_kinds gauge\n");
    out.push_str(&format!("ente_brain_distinct_kinds {}\n", obs.marginals().len()));

    // ---- Window ocupación ----
    out.push_str("# HELP ente_brain_window_size Current sliding window length.\n");
    out.push_str("# TYPE ente_brain_window_size gauge\n");
    out.push_str(&format!("ente_brain_window_size {}\n", obs.current_window()));

    // ---- Reglas vivas ----
    out.push_str("# HELP ente_brain_rules_active Number of rules currently in the engine.\n");
    out.push_str("# TYPE ente_brain_rules_active gauge\n");
    out.push_str(&format!("ente_brain_rules_active {}\n", engine.len()));

    // ---- Eventos por kind ----
    out.push_str("# HELP ente_brain_events_by_kind Events by EventKind tag.\n");
    out.push_str("# TYPE ente_brain_events_by_kind counter\n");
    for (k, c) in obs.marginals() {
        out.push_str(&format!(
            "ente_brain_events_by_kind{{kind=\"{}\"}} {}\n",
            kind_label(k), c
        ));
    }

    // ---- Cristales detectados (con params actuales) ----
    let crystals = arje_brain_cognitive::detect_crystals(&obs, &state.params);
    out.push_str("# HELP ente_brain_crystals_total Number of crystals detected with current params.\n");
    out.push_str("# TYPE ente_brain_crystals_total gauge\n");
    out.push_str(&format!("ente_brain_crystals_total {}\n", crystals.len()));

    // ---- Audit log ----
    out.push_str("# HELP ente_brain_audit_chain_length Total entries persisted to CAS.\n");
    out.push_str("# TYPE ente_brain_audit_chain_length counter\n");
    out.push_str(&format!("ente_brain_audit_chain_length {}\n", audit.flushed_count()));

    out.push_str("# HELP ente_brain_audit_in_memory Entries currently in the in-memory ring.\n");
    out.push_str("# TYPE ente_brain_audit_in_memory gauge\n");
    out.push_str(&format!("ente_brain_audit_in_memory {}\n", audit.len()));

    out.push_str("# HELP ente_brain_audit_subscribers Active stream-audit subscribers.\n");
    out.push_str("# TYPE ente_brain_audit_subscribers gauge\n");
    out.push_str(&format!("ente_brain_audit_subscribers {}\n", audit.subscriber_count()));

    if let Some(age) = audit.last_flush_age_secs() {
        out.push_str("# HELP ente_brain_audit_last_flush_age_seconds Time since last flush to CAS.\n");
        out.push_str("# TYPE ente_brain_audit_last_flush_age_seconds gauge\n");
        out.push_str(&format!("ente_brain_audit_last_flush_age_seconds {:.3}\n", age));
    }
    if let Some(sha) = audit.last_flushed_sha() {
        // Info-style metric con head sha como label. Útil para dashboards
        // que quieran mostrar "current head".
        out.push_str("# HELP ente_brain_audit_head_info Current head SHA of the audit chain.\n");
        out.push_str("# TYPE ente_brain_audit_head_info gauge\n");
        out.push_str(&format!(
            "ente_brain_audit_head_info{{sha=\"{}\"}} 1\n",
            arje_cas::hex(&sha)
        ));
    }

    // ---- Histogramas de gaps temporales (top-32 pares más frecuentes) ----
    out.push_str("# HELP ente_brain_pair_gap_seconds Time gap between correlated events.\n");
    out.push_str("# TYPE ente_brain_pair_gap_seconds histogram\n");
    let limits = arje_brain_cognitive::observer::GapHistogram::bucket_limits();
    for ((a, b), hist) in obs.top_gap_pairs(32) {
        let labels = format!(r#"a="{}",b="{}""#, kind_label(a), kind_label(b));
        for (i, &limit) in limits.iter().enumerate() {
            out.push_str(&format!(
                "ente_brain_pair_gap_seconds_bucket{{{},le=\"{}\"}} {}\n",
                labels, limit, hist.buckets[i]
            ));
        }
        out.push_str(&format!(
            "ente_brain_pair_gap_seconds_bucket{{{},le=\"+Inf\"}} {}\n",
            labels, hist.count
        ));
        out.push_str(&format!(
            "ente_brain_pair_gap_seconds_sum{{{}}} {:.6}\n",
            labels, hist.sum_secs
        ));
        out.push_str(&format!(
            "ente_brain_pair_gap_seconds_count{{{}}} {}\n",
            labels, hist.count
        ));
    }

    out
}

fn kind_label(k: &EventKind) -> &'static str {
    match k {
        EventKind::EnteSpawned => "EnteSpawned",
        EventKind::EnteDied => "EnteDied",
        EventKind::BusAnnounce => "BusAnnounce",
        EventKind::BusInvoke => "BusInvoke",
        EventKind::BusInvokeOf(_) => "BusInvokeOf",
        EventKind::DeviceAdded => "DeviceAdded",
        EventKind::DeviceRemoved => "DeviceRemoved",
        EventKind::Custom(_) => "Custom",
    }
}
