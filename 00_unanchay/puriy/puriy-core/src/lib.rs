//! puriy-core — Modelo agnóstico del navegador.
//!
//! Sesiones, tabs, history, bookmarks, perfiles. Sin deps de Servo ni de
//! Llimphi. Testeable con `cargo test`.
//!
//! Fase 1: pendiente.

/// Una pestaña abierta con su URL y título.
#[derive(Debug, Clone)]
pub struct Tab {
    pub id: u64,
    pub url: String,
    pub title: String,
}

/// Sesión = colección ordenada de tabs + tab activa.
#[derive(Debug, Default)]
pub struct Session {
    pub tabs: Vec<Tab>,
    pub active: Option<u64>,
}
