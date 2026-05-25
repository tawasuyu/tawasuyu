//! `pineal-financial` — OHLC y candlesticks.
//!
//! Layout del buffer: 6 floats por bar `[t, o, h, l, c, v]` (time,
//! open, high, low, close, volume). Mismo principio P1 del doc
//! canónico: array plano, sin objetos por bar.
//!
//! Aggregation (sección 3.2 del ARCHITECTURE.md):
//! - **Time bucketing** (no index bucketing) para que weekends /
//!   holidays no colapsen la rate.
//! - `open` = primero del bucket, `close` = último, `high` = max,
//!   `low` = min, `volume` = sum.
//! - **Preserva volatilidad** — LTTB caería los wicks; estos los
//!   conserva por construcción.
//!
//! Render: dos batches separados — barras alcistas (close > open,
//! verdes) y bajistas (close < open, rojas). v0.1 emite un quad
//! por body + un line por wick (≈ 2 draw calls por bar; aceptable
//! hasta ~500 bars on-screen). Optimización futura: agrupar
//! N bodies en un solo PathBuilder fill.

#![forbid(unsafe_code)]
#![allow(dead_code)]

pub mod ohlc_buffer;
pub mod aggregate;

#[cfg(any(feature = "gpui", feature = "llimphi"))]
pub mod candlestick;

#[cfg(feature = "gpui")]
pub mod element;

#[cfg(feature = "llimphi")]
pub mod view;

pub use ohlc_buffer::{Bar, OhlcBuffer};
pub use aggregate::aggregate_time_bucketed;

#[cfg(any(feature = "gpui", feature = "llimphi"))]
pub use candlestick::{paint_candlesticks, CandlestickStyle};

#[cfg(feature = "gpui")]
pub use element::{lapaloma_candlestick, LapalomaCandlestickElement};

#[cfg(feature = "llimphi")]
pub use view::{lapaloma_candlestick_view, CandlestickView};
