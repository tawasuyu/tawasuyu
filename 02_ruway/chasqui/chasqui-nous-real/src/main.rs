//! `chasqui-nous-real` — proveedor Nous con LLM real (gated por feature).
//!
//! ## Build modes
//!
//! - `cargo build -p chasqui-nous-real`
//!   Compila como **stub**: bin que arranca, sidecarea al brahman-init
//!   pero rechaza toda request con un error explicando que falta la
//!   feature. Útil para que `cargo build --workspace` no requiera ML
//!   deps.
//!
//! - `cargo build -p chasqui-nous-real --features embeddings`
//!   Compila con `fastembed` + ONNX Runtime descargado por Cargo.
//!   Modelo default: `all-MiniLM-L6-v2` (384-d, ~80 MB descargado al
//!   primer run y cacheado en `~/.cache/fastembed`).
//!
//! ## Diseño
//!
//! Mismo contrato wire que `chasqui-nous-mock` (`chasqui-nous` crate). La
//! diferencia operativa: real produce 384-d con semantic content
//! (text-embedding del modelo); mock produce 32-d con metadata-hashing.
//! No son intercambiables a media-deployment — los centroides de
//! Mónadas calculadas con uno NO matchean con el otro.
//!
//! La Card declara `priority_contexts.prod = { priority_offset: +1 }`,
//! contrapeso del mock que tiene `+1 en test`. Así el broker brahman
//! elige automáticamente:
//! - `BRAHMAN_BROKER_CONTEXT=test` → mock gana.
//! - `BRAHMAN_BROKER_CONTEXT=prod` → real gana.
//! - sin contexto → empate por label alfabético.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use card_core::{
    ulid::Ulid, Card, CardKind, ContextBias, Flow, Flows, Lifecycle, Payload, Priority,
    Supervision, TypeRef,
};
use chasqui_nous::{transport, FLOW_EMBED_REQUEST, FLOW_EMBED_RESULT, FLOW_TYPE_NAME};
use tokio::net::UnixListener;
use tracing::info;

#[cfg(feature = "embeddings")]
mod cache;
#[cfg(feature = "embeddings")]
mod embeddings;
#[cfg(not(feature = "embeddings"))]
mod stub;

#[cfg(feature = "embeddings")]
const MODEL_ID: &str = "real-fastembed-allMiniLML6V2-384d";
#[cfg(not(feature = "embeddings"))]
const MODEL_ID: &str = "real-stub-no-feature";

#[cfg(feature = "embeddings")]
const EMBED_DIM: u32 = 384;
#[cfg(not(feature = "embeddings"))]
const EMBED_DIM: u32 = 0;

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    init_tracing();

    #[cfg(not(feature = "embeddings"))]
    info!(
        "chasqui-nous-real corriendo en modo STUB (compilá con \
        --features embeddings para activar el modelo)"
    );

    // 1. Resolver socket del data-plane (default `chasqui-nous-real.sock`,
    //    distinto del mock para coexistir).
    let sock_path = transport::provider_socket_path("real");
    if sock_path.exists() {
        std::fs::remove_file(&sock_path)?;
    }
    if let Some(parent) = sock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(&sock_path)?;
    info!(socket = %sock_path.display(), "chasqui-nous-real escuchando");

    // 2. Sidecar al brahman-init con Card declarando el socket.
    let card = build_card(sock_path.clone());
    info!(label = %card.label, mode = MODEL_ID, "publicando Card al brahman-init");
    card_sidecar::spawn(card);

    // 3. Inicializar el modelo (sólo en modo embeddings).
    #[cfg(feature = "embeddings")]
    let backend = embeddings::Backend::init().map_err(|e| {
        std::io::Error::other(format!("init modelo: {e}"))
    })?;
    #[cfg(feature = "embeddings")]
    let backend = std::sync::Arc::new(backend);

    // 4. Abrir el cache de embeddings (sled local, sha256-keyed).
    //    Si falla, seguimos sin cache — degrada a "siempre embed".
    #[cfg(feature = "embeddings")]
    let embed_cache = match cache::EmbedCache::open() {
        Ok(c) => {
            info!(entries = c.len(), "embed-cache abierto");
            Some(c)
        }
        Err(e) => {
            tracing::warn!(error = %e, "embed-cache no disponible — todas las requests irán al modelo");
            None
        }
    };

    // 5. Accept loop.
    loop {
        let (stream, _addr) = listener.accept().await?;

        #[cfg(feature = "embeddings")]
        {
            let backend = backend.clone();
            let cache = embed_cache.clone();
            tokio::spawn(async move {
                if let Err(e) = embeddings::handle_conn(stream, backend, cache).await {
                    tracing::warn!(error = %e, "conn falló");
                }
            });
        }

        #[cfg(not(feature = "embeddings"))]
        {
            tokio::spawn(async move {
                if let Err(e) = stub::handle_conn(stream).await {
                    tracing::warn!(error = %e, "conn falló");
                }
            });
        }
    }
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .compact()
        .init();
}

/// Card que real-nous anuncia. Idéntica al mock excepto por:
/// - label distinto (`chasqui.nous_real`) para que coexistan en el broker.
/// - `priority_contexts.prod = +1` (gana en contexto prod).
/// - `service_socket` propio para que clientes lo descubran directo.
fn build_card(service_socket: std::path::PathBuf) -> Card {
    let mut priority_contexts = BTreeMap::new();
    priority_contexts.insert(
        "prod".into(),
        ContextBias {
            pin_to: None,
            priority_offset: 1,
        },
    );

    Card {
        schema_version: card_core::CARD_SCHEMA_VERSION,
        id: Ulid::new(),
        label: "chasqui.nous_real".into(),
        payload: Payload::Virtual,
        supervision: Supervision::Delegate,
        lifecycle: Lifecycle::Daemon,
        priority: Priority::Normal,
        kind: CardKind::Ente,
        service_socket: Some(service_socket),
        flow: Flows {
            input: vec![Flow {
                name: FLOW_EMBED_REQUEST.into(),
                ty: TypeRef::Primitive {
                    name: FLOW_TYPE_NAME.into(),
                },
                pin_to: None,
            }],
            output: vec![Flow {
                name: FLOW_EMBED_RESULT.into(),
                ty: TypeRef::Primitive {
                    name: FLOW_TYPE_NAME.into(),
                },
                pin_to: None,
            }],
        },
        priority_contexts,
        ..Default::default()
    }
}

// Helpers compartidos. Anotados allow(dead_code) porque en stub mode
// algunos quedan sin uso pero los queremos disponibles consistentemente.

#[allow(dead_code)]
pub(crate) fn model_id() -> &'static str {
    MODEL_ID
}

#[allow(dead_code)]
pub(crate) fn embed_dim() -> u32 {
    EMBED_DIM
}
