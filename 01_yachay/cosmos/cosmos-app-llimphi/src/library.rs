//! Biblioteca de cartas sobre `cosmos-store` (SQLite): abre el store,
//! lo siembra/migra en la primera corrida y arma un **snapshot
//! jerárquico** plano (grupo → subgrupos → contactos → cartas) que el
//! árbol izquierdo pinta como un explorador de archivos clásico.
//!
//! El árbol no consulta SQLite por frame: `snapshot()` se llama al
//! arrancar (y tras mutaciones) y deja un `Vec<NavNode>` cacheado en el
//! `Model`. Cargar una carta sí va al store por id (`get_chart`).

pub(crate) use cosmos_model::ChartKind;
use cosmos_model::{Chart, ChartId, ContactId, GroupId};
use cosmos_store::Store;

use crate::persist::{list_cards, load_card};

/// Parsea la parte `<id>` de una clave `"<prefijo>:<id>"`.
fn key_id(key: &str, prefix: &str) -> Option<String> {
    key.strip_prefix(prefix).map(|s| s.to_string())
}

pub(crate) fn parse_group_key(key: &str) -> Option<GroupId> {
    key_id(key, "g:")?.parse().ok()
}

pub(crate) fn parse_contact_key(key: &str) -> Option<ContactId> {
    key_id(key, "c:")?.parse().ok()
}

pub(crate) fn parse_chart_key(key: &str) -> Option<ChartId> {
    key_id(key, "h:")?.parse().ok()
}

/// Borra un contacto y todas sus cartas.
pub(crate) fn delete_contact_recursive(store: &Store, id: ContactId) {
    for ch in store.list_charts(id).unwrap_or_default() {
        let _ = store.delete_chart(ch.id);
    }
    let _ = store.delete_contact(id);
}

/// Borra un grupo, sus subgrupos, contactos y cartas (en cascada manual —
/// `delete_group` del store no cascadea).
pub(crate) fn delete_group_recursive(store: &Store, id: GroupId) {
    for sub in store.list_groups(Some(id)).unwrap_or_default() {
        delete_group_recursive(store, sub.id);
    }
    for c in store.list_contacts(Some(id)).unwrap_or_default() {
        delete_contact_recursive(store, c.id);
    }
    let _ = store.delete_group(id);
}

/// Tipo de nodo del árbol de datos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavKind {
    Group,
    Contact,
    Chart,
}

/// Un nodo del snapshot jerárquico, ya aplanado en orden de display con
/// su profundidad. La visibilidad real (colapsado/expandido) la resuelve
/// el árbol contra el set de nodos expandidos del `Model`.
#[derive(Debug, Clone)]
pub(crate) struct NavNode {
    /// Clave única y estable: `"g:<id>"`, `"c:<id>"`, `"h:<id>"`.
    pub(crate) key: String,
    /// Clave del padre (grupo o contacto). `None` = raíz.
    pub(crate) parent: Option<String>,
    pub(crate) depth: usize,
    pub(crate) label: String,
    pub(crate) kind: NavKind,
    /// Id de la carta (sólo en nodos `Chart`) para `get_chart`.
    pub(crate) chart_id: Option<String>,
    /// Tipo de carta (sólo en nodos `Chart`) — define su icono en el árbol.
    pub(crate) chart_kind: Option<ChartKind>,
}

/// Abre (o crea) el store SQLite en el config dir de wawa. `None` si no
/// hay config dir o SQLite falla — el árbol queda vacío pero la app sigue.
pub(crate) fn open_store() -> Option<Store> {
    let path = wawa_config::config_dir()?.join("cosmos.db");
    Store::open(&path)
        .map_err(|e| eprintln!("cosmos · store: no se pudo abrir {path:?}: {e}"))
        .ok()
}

/// Siembra el store si está vacío: migra las cartas JSON existentes
/// (`cosmos-charts/*.json`) bajo un grupo «Cartas» / contacto
/// «Importadas»; si no hay ninguna, crea una de ejemplo desde `fallback`.
pub(crate) fn ensure_seed(store: &Store, fallback: &Chart) {
    let empty = store
        .list_groups(None)
        .map(|g| g.is_empty())
        .unwrap_or(true)
        && store
            .list_all_charts()
            .map(|c| c.is_empty())
            .unwrap_or(true);
    if !empty {
        return;
    }

    let group = match store.create_group(None, "Cartas", None) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("cosmos · store: seed grupo: {e}");
            return;
        }
    };
    let contact = match store.create_contact(Some(group.id), "Importadas", None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cosmos · store: seed contacto: {e}");
            return;
        }
    };

    // Migrar la biblioteca JSON existente.
    let mut migradas = 0usize;
    for name in list_cards() {
        if let Some(ch) = load_card(&name) {
            if store
                .create_chart(
                    contact.id,
                    ChartKind::Natal,
                    &ch.label,
                    &ch.birth_data,
                    &ch.config,
                    None,
                )
                .is_ok()
            {
                migradas += 1;
            }
        }
    }

    // Si no había nada que migrar, sembrar la carta actual de ejemplo.
    if migradas == 0 {
        let _ = store.create_chart(
            contact.id,
            ChartKind::Natal,
            &fallback.label,
            &fallback.birth_data,
            &fallback.config,
            None,
        );
    }
}

/// Arma el snapshot jerárquico completo (grupos anidados → contactos →
/// cartas) en orden de display.
pub(crate) fn snapshot(store: &Store) -> Vec<NavNode> {
    let mut out = Vec::new();
    walk_groups(store, None, None, 0, &mut out);
    // Contactos sin grupo, a la raíz.
    add_contacts(store, None, None, 0, &mut out);
    out
}

fn walk_groups(
    store: &Store,
    parent_id: Option<GroupId>,
    parent_key: Option<String>,
    depth: usize,
    out: &mut Vec<NavNode>,
) {
    let groups = store.list_groups(parent_id).unwrap_or_default();
    for g in groups {
        let gkey = format!("g:{}", g.id);
        out.push(NavNode {
            key: gkey.clone(),
            parent: parent_key.clone(),
            depth,
            label: g.name.clone(),
            kind: NavKind::Group,
            chart_id: None,
            chart_kind: None,
        });
        // Subgrupos primero, luego contactos del grupo.
        walk_groups(store, Some(g.id), Some(gkey.clone()), depth + 1, out);
        add_contacts(store, Some(g.id), Some(gkey.clone()), depth + 1, out);
    }
}

fn add_contacts(
    store: &Store,
    group_id: Option<GroupId>,
    parent_key: Option<String>,
    depth: usize,
    out: &mut Vec<NavNode>,
) {
    let contacts = store.list_contacts(group_id).unwrap_or_default();
    for c in contacts {
        let ckey = format!("c:{}", c.id);
        out.push(NavNode {
            key: ckey.clone(),
            parent: parent_key.clone(),
            depth,
            label: c.name.clone(),
            kind: NavKind::Contact,
            chart_id: None,
            chart_kind: None,
        });
        let charts = store.list_charts(c.id).unwrap_or_default();
        for ch in charts {
            out.push(NavNode {
                key: format!("h:{}", ch.id),
                parent: Some(ckey.clone()),
                depth: depth + 1,
                label: ch.label.clone(),
                kind: NavKind::Chart,
                chart_id: Some(ch.id.to_string()),
                chart_kind: Some(ch.kind),
            });
        }
    }
}

/// Claves de todos los nodos contenedores (grupos + contactos) — usado
/// para expandir todo en la primera carga.
pub(crate) fn container_keys(nodes: &[NavNode]) -> Vec<String> {
    nodes
        .iter()
        .filter(|n| n.kind != NavKind::Chart)
        .map(|n| n.key.clone())
        .collect()
}
