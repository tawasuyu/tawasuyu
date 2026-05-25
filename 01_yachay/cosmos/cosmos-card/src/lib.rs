//! `cosmos_app-card` — Tarjeta de Presentación + sidecar de la app.
//!
//! Cualquier binario que levante Tahuantinsuyu llama [`spawn_sidecar`]
//! antes de abrir la ventana GPUI. La lógica de thread / tokio /
//! ping-loop vive en `brahman-sidecar`; aquí solo declaramos quién es
//! Tahuantinsuyu como módulo Brahman.

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

pub mod service;

use std::collections::BTreeSet;

use card_core::{
    Card, Flow, Flows, FsPolicy, IpcPolicy, Lifecycle, Payload, Permissions, Priority, Supervision,
    TypeRef, CARD_SCHEMA_VERSION,
};
use ulid::Ulid;

/// Label canónico — coincide con el binario y aparece en `ListEntes`.
pub const LABEL: &str = "brahman.cosmos_app";

/// Spawn fire-and-forget. Si el Init no está corriendo, el sidecar
/// loggea y termina; la app sigue ejecutándose standalone.
pub fn spawn_sidecar() {
    card_sidecar::spawn(build_card());
}

/// Construye la Card. Expuesto público para tests + para shells que
/// quieran inspeccionar el manifiesto antes de spawnear. Anuncia el
/// path del service socket en `Card.service_socket` para que otros
/// módulos brahman, después de matchear via el broker, puedan conectar
/// directo al data plane.
pub fn build_card() -> Card {
    Card {
        schema_version: CARD_SCHEMA_VERSION,
        id: Ulid::new(),
        lineage: None,
        label: LABEL.into(),
        service_socket: Some(service::default_service_socket()),
        provides: BTreeSet::new(),
        requires: BTreeSet::new(),
        payload: Payload::Virtual,
        supervision: Supervision::Delegate,
        lifecycle: Lifecycle::Widget,
        priority: Priority::Normal,
        permissions: Permissions {
            // La app guarda su DB SQLite en disco; necesita RW filesystem.
            filesystem: FsPolicy::ReadWrite,
            ipc: IpcPolicy {
                allow: vec!["wit-v1".into()],
            },
            ..Default::default()
        },
        flow: Flows {
            // Recibe peticiones de cómputo (carta natal, transit, etc.)
            // serializadas como JSON. La forma exacta la define
            // `cosmos_app-engine`.
            input: vec![Flow {
                name: "chart-request".into(),
                ty: TypeRef::Primitive {
                    name: "json".into(),
                },
                pin_to: None,
            }],
            // Publica el resultado de un cómputo (placements, aspectos,
            // casas) también como JSON. Otras apps brahman pueden
            // consumirlo para visualizar o derivar.
            output: vec![Flow {
                name: "chart-result".into(),
                ty: TypeRef::Primitive {
                    name: "json".into(),
                },
                pin_to: None,
            }],
        },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_label_and_flow() {
        let c = build_card();
        assert_eq!(c.label, LABEL);
        assert_eq!(c.flow.input.len(), 1);
        assert_eq!(c.flow.output.len(), 1);
        assert_eq!(c.flow.input[0].name, "chart-request");
        assert_eq!(c.flow.output[0].name, "chart-result");
    }
}
