//! `matilda-plan` — reconciliación de estado deseado vs actual.
//!
//! Dado el inventario *actual* de un servidor y el inventario *deseado*,
//! produce la lista de [`Action`]s que lo lleva de uno al otro. El orden
//! respeta las dependencias:
//!
//! 1. crear/actualizar hosts;
//! 2. crear/actualizar contenedores (los vhosts dependen de ellos);
//! 3. crear/actualizar vhosts;
//! 4. eliminar vhosts (antes que sus contenedores);
//! 5. eliminar contenedores;
//! 6. eliminar hosts.
//!
//! Es una función pura y determinista — el mismo par de inventarios da
//! siempre el mismo plan. Aplicarlo (Docker, nginx, SSH) es trabajo de
//! capas superiores.

#![forbid(unsafe_code)]

use matilda_core::Inventory;
use serde::{Deserialize, Serialize};

/// El tipo de recurso sobre el que opera una acción.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Resource {
    Host,
    Container,
    VHost,
}

impl Resource {
    fn label(self) -> &'static str {
        match self {
            Resource::Host => "host",
            Resource::Container => "contenedor",
            Resource::VHost => "vhost",
        }
    }
}

/// La operación de una acción.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    Create,
    Update,
    Remove,
}

impl Op {
    fn verb(self) -> &'static str {
        match self {
            Op::Create => "crear",
            Op::Update => "actualizar",
            Op::Remove => "eliminar",
        }
    }
}

/// Una acción del plan: operar sobre un recurso identificado por nombre.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub op: Op,
    pub resource: Resource,
    /// Nombre del recurso — `name` del host/contenedor, `domain` del vhost.
    pub name: String,
}

impl Action {
    fn new(op: Op, resource: Resource, name: impl Into<String>) -> Self {
        Self { op, resource, name: name.into() }
    }

    /// Descripción legible — `"crear contenedor «web»"`.
    pub fn describe(&self) -> String {
        format!("{} {} «{}»", self.op.verb(), self.resource.label(), self.name)
    }
}

/// El plan de reconciliación: acciones en orden de aplicación.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub actions: Vec<Action>,
}

impl Plan {
    /// `true` si no hay nada que cambiar — los inventarios ya coinciden.
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Cantidad de acciones.
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    /// Cuenta las acciones de una operación dada.
    pub fn count(&self, op: Op) -> usize {
        self.actions.iter().filter(|a| a.op == op).count()
    }
}

/// Calcula el plan que lleva de `current` a `desired`.
pub fn plan(current: &Inventory, desired: &Inventory) -> Plan {
    let mut actions: Vec<Action> = Vec::new();

    // --- Fase 1: hosts a crear/actualizar ---
    for h in desired.hosts() {
        match current.host(&h.name) {
            None => actions.push(Action::new(Op::Create, Resource::Host, &h.name)),
            Some(cur) if cur != h => {
                actions.push(Action::new(Op::Update, Resource::Host, &h.name))
            }
            Some(_) => {}
        }
    }

    // --- Fase 2: contenedores a crear/actualizar ---
    for c in desired.containers() {
        match current.container(&c.name) {
            None => actions.push(Action::new(Op::Create, Resource::Container, &c.name)),
            Some(cur) if cur != c => {
                actions.push(Action::new(Op::Update, Resource::Container, &c.name))
            }
            Some(_) => {}
        }
    }

    // --- Fase 3: vhosts a crear/actualizar ---
    for v in desired.vhosts() {
        match current.vhost(&v.domain) {
            None => actions.push(Action::new(Op::Create, Resource::VHost, &v.domain)),
            Some(cur) if cur != v => {
                actions.push(Action::new(Op::Update, Resource::VHost, &v.domain))
            }
            Some(_) => {}
        }
    }

    // --- Fase 4: vhosts a eliminar (antes que sus contenedores) ---
    for v in current.vhosts() {
        if desired.vhost(&v.domain).is_none() {
            actions.push(Action::new(Op::Remove, Resource::VHost, &v.domain));
        }
    }

    // --- Fase 5: contenedores a eliminar ---
    for c in current.containers() {
        if desired.container(&c.name).is_none() {
            actions.push(Action::new(Op::Remove, Resource::Container, &c.name));
        }
    }

    // --- Fase 6: hosts a eliminar ---
    for h in current.hosts() {
        if desired.host(&h.name).is_none() {
            actions.push(Action::new(Op::Remove, Resource::Host, &h.name));
        }
    }

    Plan { actions }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matilda_core::{Container, Host, VHost};

    #[test]
    fn empty_to_empty_is_a_noop() {
        let p = plan(&Inventory::new(), &Inventory::new());
        assert!(p.is_empty());
    }

    #[test]
    fn fresh_inventory_is_all_creates() {
        let mut desired = Inventory::new();
        desired.add_host(Host::new("edge", "10.0.0.1"));
        desired.add_container(Container::new("web", "nginx"));
        desired.add_vhost(VHost::to_container("site.com", "web", 80));
        let p = plan(&Inventory::new(), &desired);
        assert_eq!(p.count(Op::Create), 3);
        assert_eq!(p.count(Op::Remove), 0);
    }

    #[test]
    fn unchanged_inventory_yields_no_actions() {
        let mut inv = Inventory::new();
        inv.add_container(Container::new("web", "nginx:1.27"));
        let p = plan(&inv, &inv.clone());
        assert!(p.is_empty());
    }

    #[test]
    fn changed_image_is_an_update() {
        let mut current = Inventory::new();
        current.add_container(Container::new("web", "nginx:1.26"));
        let mut desired = Inventory::new();
        desired.add_container(Container::new("web", "nginx:1.27"));
        let p = plan(&current, &desired);
        assert_eq!(p.actions, vec![Action::new(Op::Update, Resource::Container, "web")]);
    }

    #[test]
    fn dropped_resources_become_removes() {
        let mut current = Inventory::new();
        current.add_container(Container::new("old", "img"));
        current.add_vhost(VHost::to_container("old.com", "old", 80));
        let p = plan(&current, &Inventory::new());
        assert_eq!(p.count(Op::Remove), 2);
    }

    #[test]
    fn vhost_removal_precedes_container_removal() {
        // Un vhost debe eliminarse antes que el contenedor que lo sirve.
        let mut current = Inventory::new();
        current.add_container(Container::new("web", "nginx"));
        current.add_vhost(VHost::to_container("site.com", "web", 80));
        let p = plan(&current, &Inventory::new());
        let vhost_pos = p
            .actions
            .iter()
            .position(|a| a.resource == Resource::VHost)
            .unwrap();
        let cont_pos = p
            .actions
            .iter()
            .position(|a| a.resource == Resource::Container)
            .unwrap();
        assert!(vhost_pos < cont_pos);
    }

    #[test]
    fn container_creation_precedes_vhost_creation() {
        let mut desired = Inventory::new();
        desired.add_container(Container::new("web", "nginx"));
        desired.add_vhost(VHost::to_container("site.com", "web", 80));
        let p = plan(&Inventory::new(), &desired);
        let cont_pos = p
            .actions
            .iter()
            .position(|a| a.resource == Resource::Container)
            .unwrap();
        let vhost_pos = p
            .actions
            .iter()
            .position(|a| a.resource == Resource::VHost)
            .unwrap();
        assert!(cont_pos < vhost_pos);
    }

    #[test]
    fn plan_is_deterministic() {
        let mut current = Inventory::new();
        current.add_container(Container::new("a", "img:1"));
        let mut desired = Inventory::new();
        desired.add_container(Container::new("a", "img:2"));
        desired.add_container(Container::new("b", "img:1"));
        assert_eq!(plan(&current, &desired), plan(&current, &desired));
    }

    #[test]
    fn describe_is_human_readable() {
        let a = Action::new(Op::Create, Resource::Container, "web");
        assert_eq!(a.describe(), "crear contenedor «web»");
    }
}
