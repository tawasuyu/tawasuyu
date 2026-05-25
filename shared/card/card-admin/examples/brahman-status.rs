//! `brahman-status` — CLI para inspeccionar el estado del Init.
//!
//! Conecta al socket admin (default `$XDG_RUNTIME_DIR/brahman-admin.sock`,
//! override con `$BRAHMAN_ADMIN_SOCKET`), recibe el snapshot, y lo imprime.

use card_admin::{client, transport};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let path = transport::default_socket_path();
    let snap = client::query(&path).await?;

    println!(
        "Init: server={} protocol={} attached={}",
        snap.server_version, snap.protocol_version, snap.init_attached
    );
    if let Some(ctx) = &snap.current_context {
        println!("Context: {}", ctx);
    }
    println!();
    println!("Sessions ({}):", snap.sessions.len());
    if snap.sessions.is_empty() {
        println!("  (ninguna)");
    } else {
        for s in &snap.sessions {
            let conscious_marker = if s.wit.is_some() { " 🧠" } else { "" };
            let kind_marker = match s.kind {
                card_core::CardKind::Ente => "ente",
                card_core::CardKind::Data => "data",
            };
            println!(
                "  [{}]  {}  {}{}  lifecycle={:?}  priority={:?}",
                kind_marker, s.session, s.label, conscious_marker, s.lifecycle, s.priority
            );
            if let Some(sock) = &s.service_socket {
                println!("      socket:     {}", sock.display());
            }
            for r in &s.references {
                println!(
                    "      ref {:?}  →  {}  ({})",
                    r.kind, r.target_label, r.target_id
                );
            }
            if let Some(data) = &s.data {
                if !data.summary.is_empty() {
                    println!("      summary:    {}", data.summary);
                }
                if data.member_count > 0 {
                    println!(
                        "      members:    {} (dispersion={:.2})",
                        data.member_count, data.dispersion
                    );
                }
                if !data.keywords.is_empty() {
                    println!("      keywords:   {}", data.keywords.join(", "));
                }
                if !data.presentation_hint.is_empty() {
                    println!("      lens hint:  {}", data.presentation_hint);
                }
            }
            if let Some(wit) = &s.wit {
                println!("      wit: {} / {}", wit.package, wit.world);
                if !wit.imports.is_empty() {
                    println!("           imports: {}", wit.imports.join(", "));
                }
                if !wit.exports.is_empty() {
                    println!("           exports: {}", wit.exports.join(", "));
                }
            }
            for f in &s.inputs {
                println!("      in  {}: {:?}", f.name, f.ty);
            }
            for f in &s.outputs {
                println!("      out {}: {:?}", f.name, f.ty);
            }
        }
    }
    println!();
    println!("Matches ({}):", snap.matches.len());
    if snap.matches.is_empty() {
        println!("  (ninguno)");
    } else {
        for m in &snap.matches {
            let pin_marker = if m.pinned { "📌" } else { "  " };
            println!(
                "  {} {}.{}  ←  {}.{}  via {:?}",
                pin_marker,
                m.consumer_label,
                m.consumer.flow_name,
                m.producer_label,
                m.producer.flow_name,
                m.via
            );
        }
    }

    Ok(())
}
