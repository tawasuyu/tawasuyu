//! ente-echo: Ente proveedor mínimo. Anuncia Capability::Endpoint(ECHO) y
//! responde a invokes echando el blob recibido. Vehículo para validar el
//! forwarding bus → proveedor → bus → originator.

use arje_bus::{BusResponse, BusServer, InvokeHandler};
use arje_card::Capability;
use arje_echo::{echo_capability, ECHO_IFACE, ECHO_VERSION};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

struct EchoHandler;

impl InvokeHandler for EchoHandler {
    fn handle(&mut self, cap: Capability, blob: Vec<u8>) -> BusResponse {
        match cap {
            Capability::Endpoint { interface, version }
                if interface == ECHO_IFACE && version == ECHO_VERSION =>
            {
                let preview = String::from_utf8_lossy(&blob).into_owned();
                info!(text = %preview, len = blob.len(), "echo invoke");
                BusResponse::Invoked { result: blob }
            }
            other => {
                warn!(?other, "ente-echo: capacidad no soportada");
                BusResponse::Error("ente-echo solo maneja ECHO_IFACE".into())
            }
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    bitacora::abrir("arje");
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("arje_echo=info"));
    // try_init: bitacora::abrir ya puede haber instalado el subscriber global.
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_target(true).try_init();

    info!("ente-echo arrancando");
    let mut server = BusServer::from_env().await?;
    server.announce(vec![echo_capability()]).await?;
    info!("Announce OK, sirviendo invokes");

    if let Err(e) = server.serve(EchoHandler).await {
        warn!(?e, "serve terminó");
    }
    Ok(())
}
