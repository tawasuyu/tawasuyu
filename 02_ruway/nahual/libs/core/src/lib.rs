//! `nahual_core` — tipos compartidos por toda la app, sin dependencias de UI.
//!
//! Contiene tres bloques:
//! 1. **Providers** (`DataProvider`, `EntityNode`, `DisplayType`) — fuente de
//!    datos jerárquicos para los exploradores. Portado intacto de `gioser_core`.
//! 2. **Layout** (`LayerConfig`, `LayerParam`, `ModuloTipo`, `LayoutDirection`,
//!    `LayoutMode`) — el JSON describe el árbol de widgets. Portado de
//!    `gioser_core` quitando los helpers acoplados a Makepad (`LiveId`).
//! 3. **Identidad** (`NodeId`) — id estable de un nodo del layout, derivado
//!    del `id` JSON o del path estructural.
//!
//! NO contiene tipos de comunicación entre widgets. Esos viven en la `shell`
//! y se construyen sobre el sistema de eventos de GPUI (`EventEmitter`,
//! `cx.subscribe`).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use tokio::io::{AsyncRead, AsyncWrite};

// =====================================================================
// Providers
// =====================================================================

#[derive(Clone, Debug)]
pub enum DisplayType {
    Folder,
    File,
    Stream,
}

#[derive(Clone, Debug)]
pub struct EntityNode {
    pub id: String,
    pub name: String,
    pub display_type: DisplayType,
    pub mime_type: Option<String>,
}

#[async_trait]
pub trait DataProvider: Send + Sync {
    fn provider_id(&self) -> String;

    async fn list_children(&self, parent_id: Option<&str>) -> Result<Vec<EntityNode>, String>;

    async fn get_read_stream(
        &self,
        entity_id: &str,
    ) -> Result<Pin<Box<dyn AsyncRead + Send>>, String>;

    async fn get_write_stream(
        &self,
        entity_id: &str,
    ) -> Result<Pin<Box<dyn AsyncWrite + Send>>, String>;

    /// Default convenience: vacía un read stream a `Vec<u8>`. Los providers
    /// pueden override si tienen un fast-path.
    async fn get_data(&self, entity_id: &str) -> Result<Vec<u8>, String> {
        use tokio::io::AsyncReadExt;
        let mut stream = self.get_read_stream(entity_id).await?;
        let mut buffer = Vec::new();
        stream
            .read_to_end(&mut buffer)
            .await
            .map_err(|e| e.to_string())?;
        Ok(buffer)
    }
}

// =====================================================================
// Identidad estable de nodos del layout
// =====================================================================

/// Identificador estable de un nodo del árbol de layout. Construido con
/// `NodeId::from_layer(&LayerConfig, path)` durante el DFS del LayoutHost:
/// si el `LayerConfig` trae `id` propio, se usa ese; si no, se sintetiza a
/// partir del path estructural (`root/main/0`).
///
/// Internamente es una `String` para no atarse al sistema de hashing de
/// ningún framework. La igualdad lexicográfica garantiza estabilidad: el
/// mismo `id` o el mismo path producen el mismo `NodeId`.
#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeId(pub String);

impl NodeId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn from_layer(cfg: &LayerConfig, path: &str) -> Self {
        match &cfg.id {
            Some(id) if !id.is_empty() => Self(id.clone()),
            _ => Self(path.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// =====================================================================
// Taxonomía y layout JSON
// =====================================================================

/// Tipo de módulo que la Shell sabe instanciar. Cualquier `kind` del JSON se
/// resuelve a uno de estos via `ModuloTipo::from_kind`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ModuloTipo {
    Texto,
    Arbol,
    Imagen,
    /// Marco contenedor — define un sub-layout y aloja hijos.
    Contenedor,
    /// Tile manager autónomo (Tiled / Floating / Stacked + shortcuts).
    TileManager,
}

impl ModuloTipo {
    pub fn from_kind(kind: &str) -> Self {
        match kind {
            "TextViewer" | "SectionEditor" | "Texto" => Self::Texto,
            "FileExplorer" | "DatabaseExplorer" | "Arbol" => Self::Arbol,
            "ImageViewer" | "Imagen" => Self::Imagen,
            "TileManager" | "Tiled" => Self::TileManager,
            _ => Self::Contenedor,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LayoutDirection {
    Horizontal,
    Vertical,
    Overlay,
}

impl Default for LayoutDirection {
    fn default() -> Self {
        Self::Vertical
    }
}

impl LayoutDirection {
    pub fn from_str(s: &str) -> Self {
        match s {
            "horizontal" | "Horizontal" | "row" => Self::Horizontal,
            "overlay" | "Overlay" | "stack" => Self::Overlay,
            _ => Self::Vertical,
        }
    }
}

/// Política global del root: cómo se presentan los hijos directos del
/// `LayerConfig` raíz entre sí (Tiled / Stacked / Floating). Distinta de
/// `LayoutDirection` que es por contenedor.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LayoutMode {
    Tiled,
    Stacked,
    Floating,
}

impl Default for LayoutMode {
    fn default() -> Self {
        Self::Tiled
    }
}

impl LayoutMode {
    pub fn from_str(s: &str) -> Self {
        match s {
            "stacked" | "Stacked" => Self::Stacked,
            "floating" | "Floating" => Self::Floating,
            _ => Self::Tiled,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayerParam {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayerConfig {
    /// Identificador estable. Si falta, el LayoutHost sintetiza desde el path.
    pub id: Option<String>,
    /// Nombre de clase del módulo (e.g. "FileExplorer", "Split", "Tabs").
    pub kind: String,
    /// Peso flex relativo entre hermanos. `None` ⇒ 1.0.
    pub flex: Option<f64>,
    /// Solo válido para contenedores con orientación. `None` ⇒ Vertical.
    pub direction: Option<String>,
    pub params: Vec<LayerParam>,
    pub children: Vec<LayerConfig>,
}

impl Default for LayerConfig {
    fn default() -> Self {
        Self {
            id: Some("root".to_string()),
            kind: "Split".to_string(),
            flex: Some(1.0),
            direction: Some("horizontal".to_string()),
            params: vec![],
            children: vec![
                LayerConfig {
                    id: Some("nav".to_string()),
                    kind: "FileExplorer".to_string(),
                    flex: Some(0.3),
                    direction: None,
                    params: vec![],
                    children: vec![],
                },
                LayerConfig {
                    id: Some("main".to_string()),
                    kind: "TextViewer".to_string(),
                    flex: Some(0.7),
                    direction: None,
                    params: vec![],
                    children: vec![],
                },
            ],
        }
    }
}

impl LayerConfig {
    pub fn load_or_default(path: &str) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    pub fn serialize_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn get_param(&self, key: &str) -> Option<&String> {
        self.params.iter().find(|p| p.key == key).map(|p| &p.value)
    }

    pub fn flex_weight(&self) -> f64 {
        self.flex.unwrap_or(1.0).max(0.0)
    }

    pub fn modulo_tipo(&self) -> ModuloTipo {
        ModuloTipo::from_kind(&self.kind)
    }

    pub fn layout_direction(&self) -> LayoutDirection {
        self.direction
            .as_deref()
            .map(LayoutDirection::from_str)
            .unwrap_or(LayoutDirection::Vertical)
    }
}
