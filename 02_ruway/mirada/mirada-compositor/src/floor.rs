//! Anuncio de readiness del "piso" gráfico al Init (arje).
//!
//! Cuando el socket Wayland ya escucha (los clientes PUEDEN conectar), mirada le
//! anuncia a arje —si corre como Ente del Init— que provee el piso
//! (`wayland_floor()`), vía `UpdateCapabilities`. Recién entonces el Init deja
//! arrancar (o re-floorea) a los clientes de sesión que dependen del piso, sin la
//! carrera de "arrancar antes de que el socket escuche". El Init dispara su
//! drenaje de re-floor al recibir el `UpdateCapabilities` (readiness).
//!
//! Best-effort y NO bloqueante: el bucle del compositor es `calloop` (no async),
//! así que el anuncio corre en un hilo aparte con un runtime tokio mínimo. Sin
//! bus (corrida a mano, sin `ENTE_BUS_SOCK`) o ante cualquier fallo, sólo se
//! loguea — nunca tumba al compositor.

use arje_bus::{BusClient, BusRequest, BusResponse};
use arje_card::wayland_floor;

/// Anuncia el piso al Init en segundo plano. Idempotente desde el lado del Init
/// (registrar dos veces la misma capability dinámica es inocuo). No-op si no
/// somos un Ente del Init.
pub fn announce_floor_ready() {
    if std::env::var_os("ENTE_BUS_SOCK").is_none() {
        // Corrida fuera del Init (dev / nested): no hay a quién anunciarle.
        return;
    }
    std::thread::spawn(|| {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("mirada: runtime para anunciar el piso falló — sigo: {e}");
                return;
            }
        };
        rt.block_on(async {
            match announce().await {
                Ok(()) => eprintln!("mirada: piso Wayland anunciado al Init (readiness)"),
                Err(e) => eprintln!("mirada: no pude anunciar el piso — sigo: {e}"),
            }
        });
    });
}

async fn announce() -> anyhow::Result<()> {
    let mut client = BusClient::from_env().await?;
    match client
        .call(BusRequest::UpdateCapabilities {
            adds: vec![wayland_floor()],
            removes: vec![],
        })
        .await?
    {
        BusResponse::Ok => Ok(()),
        other => anyhow::bail!("el bus rechazó UpdateCapabilities del piso: {other:?}"),
    }
}
