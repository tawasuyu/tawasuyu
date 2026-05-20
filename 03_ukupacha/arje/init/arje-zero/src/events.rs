//! Eventos internos del bucle primordial. Todo cambio de estado del fractal
//! pasa por aquí — la única vía de mutación del grafo desde tasks externas.
//!
//! Este módulo es **vocabulario**: declara el universo completo de eventos
//! del fractal. Algunas variantes/campos están reservados para flujos
//! aún no implementados (capabilities, signal-driven shutdown). Silenciar
//! `dead_code` evita ruido sin perder la declaración del contrato.

#![allow(dead_code)]

use arje_bus::{BusMessage, BusRequest, BusResponse, PeerCreds};
use arje_card::{Capability, EntityCard};
use nix::sys::signal::Signal;
use tokio::sync::{mpsc, oneshot};
use ulid::Ulid;

#[derive(Debug)]
pub enum GraphEvent {
    EnteDied { id: Ulid, status: ExitStatus },
    CapabilityRequested {
        from: Ulid,
        cap: Capability,
        reply: oneshot::Sender<CapabilityGrant>,
    },
    SpawnRequest { card: EntityCard, requester: Ulid },
    /// Request del bus interno. `peer` es no-falsificable (kernel-injected
    /// via SO_PEERCRED). `from` es la identidad reclamada por el cliente —
    /// el grafo la verifica contra `peer.pid`.
    BusRequest {
        peer: PeerCreds,
        from: Option<Ulid>,
        request: BusRequest,
        outbound: mpsc::Sender<BusMessage>,
        reply: oneshot::Sender<BusResponse>,
    },
    /// Response a un Invoke forwardeado por el grafo a un proveedor.
    /// `seq` debe coincidir con una entry en pending_invokes.
    BusResponse { seq: u64, response: BusResponse },
    /// Cliente del bus cerró su conexión. Si había anunciado identidad,
    /// el grafo retira esa conexión del registry.
    BusConnClosed { ente_id: Option<Ulid> },
    Shutdown { reason: ShutdownReason },
}

#[derive(Debug, Clone)]
pub enum ExitStatus {
    Exit(i32),
    Killed(Signal),
}

#[derive(Debug, Clone)]
pub enum ShutdownReason {
    SeedRequested,
    Signal(Signal),
}

#[derive(Debug)]
pub enum CapabilityGrant {
    Granted { token: u64 },
    NoProvider,
    Denied { reason: &'static str },
    /// El holder ya tiene el máximo de tokens activos para esta cap.
    /// Debe esperar a que alguno expire o renovar uno existente.
    QuotaExceeded { active: u32, limit: u32 },
}
