//! `pineal-stream` — telemetría streaming tipo osciloscopio.
//!
//! Núcleo: `pineal_core::ring::RingBuffer` + render en dos
//! segmentos split-at-head (modo sweep). El emisor de samples vive
//! afuera del Element — típicamente en el `Render` host con un
//! timer `cx.background_executor().timer(...)` que llama a
//! `buffer.push(value)` y `cx.notify()` cada N ms.
//!
//! El Element clona el RingBuffer por frame (para cap = 512 son
//! 4 KB, irrelevante). Para capacidades grandes (100k+) la siguiente
//! optimización es pasar `Arc<RingBuffer>` con shared read y
//! mutación interna via `Mutex`/`AtomicU64` para el head.

#![forbid(unsafe_code)]
#![allow(dead_code)]

pub mod envelope {}

pub mod view;

pub use view::{pineal_stream_view, StreamView};
