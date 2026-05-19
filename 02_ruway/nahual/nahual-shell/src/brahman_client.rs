//! Card de nahual-shell + spawn del sidecar brahman compartido.
//!
//! La lógica de thread + tokio + ping-loop vive en `brahman-sidecar`;
//! aquí sólo declaramos la identidad de nahual como módulo Widget.

use std::collections::BTreeSet;

use brahman_card::{
    Card, Flow, Flows, FsPolicy, IpcPolicy, Lifecycle, Payload, Permissions, Priority, Supervision,
    TypeRef, CARD_SCHEMA_VERSION,
};
use ulid::Ulid;

/// Spawn del sidecar con la Card de nahual.
pub fn spawn() {
    brahman_sidecar::spawn(build_card());
}

fn build_card() -> Card {
    Card {
        schema_version: CARD_SCHEMA_VERSION,
        id: Ulid::new(),
        lineage: None,
        label: "brahman.ui_engine".into(),
        provides: BTreeSet::new(),
        requires: BTreeSet::new(),
        payload: Payload::Virtual,
        supervision: Supervision::Delegate,
        lifecycle: Lifecycle::Widget,
        priority: Priority::Normal,
        permissions: Permissions {
            filesystem: FsPolicy::ReadWrite,
            ipc: IpcPolicy {
                allow: vec!["wit-v1".into()],
            },
            ..Default::default()
        },
        flow: Flows {
            input: vec![Flow {
                name: "render-data".into(),
                ty: TypeRef::Primitive {
                    name: "json".into(),
                },
                pin_to: None,
            }],
            output: vec![Flow {
                name: "user-intent".into(),
                ty: TypeRef::Primitive {
                    name: "json".into(),
                },
                pin_to: None,
            }],
        },
        ..Default::default()
    }
}
