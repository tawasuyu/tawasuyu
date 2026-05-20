//! `Inventory` — el estado declarado de la infraestructura.
//!
//! Reúne hosts, contenedores y vhosts. Cada colección es un `BTreeMap`
//! por nombre: toda iteración es determinista y el `diff` de
//! `matilda-plan` produce siempre el mismo orden de acciones.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::container::Container;
use crate::host::Host;
use crate::vhost::VHost;

/// El inventario completo — la fuente de verdad declarativa.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Inventory {
    hosts: BTreeMap<String, Host>,
    containers: BTreeMap<String, Container>,
    vhosts: BTreeMap<String, VHost>,
}

impl Inventory {
    pub fn new() -> Self {
        Self::default()
    }

    // --- Hosts ---

    pub fn add_host(&mut self, host: Host) {
        self.hosts.insert(host.name.clone(), host);
    }

    pub fn host(&self, name: &str) -> Option<&Host> {
        self.hosts.get(name)
    }

    pub fn hosts(&self) -> impl Iterator<Item = &Host> {
        self.hosts.values()
    }

    // --- Contenedores ---

    pub fn add_container(&mut self, container: Container) {
        self.containers.insert(container.name.clone(), container);
    }

    pub fn container(&self, name: &str) -> Option<&Container> {
        self.containers.get(name)
    }

    pub fn containers(&self) -> impl Iterator<Item = &Container> {
        self.containers.values()
    }

    // --- VHosts ---

    pub fn add_vhost(&mut self, vhost: VHost) {
        self.vhosts.insert(vhost.domain.clone(), vhost);
    }

    pub fn vhost(&self, domain: &str) -> Option<&VHost> {
        self.vhosts.get(domain)
    }

    pub fn vhosts(&self) -> impl Iterator<Item = &VHost> {
        self.vhosts.values()
    }

    // --- Consultas transversales ---

    /// `true` si el inventario no tiene nada declarado.
    pub fn is_empty(&self) -> bool {
        self.hosts.is_empty() && self.containers.is_empty() && self.vhosts.is_empty()
    }

    /// VHosts cuyo upstream apunta a un contenedor inexistente — la
    /// inconsistencia más común de un inventario.
    pub fn broken_vhosts(&self) -> Vec<&VHost> {
        self.vhosts
            .values()
            .filter(|v| {
                v.depends_on_container()
                    .is_some_and(|c| !self.containers.contains_key(c))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_query_each_kind() {
        let mut inv = Inventory::new();
        inv.add_host(Host::new("edge", "10.0.0.1"));
        inv.add_container(Container::new("web", "nginx:1.27"));
        inv.add_vhost(VHost::to_container("site.com", "web", 80));
        assert!(inv.host("edge").is_some());
        assert!(inv.container("web").is_some());
        assert!(inv.vhost("site.com").is_some());
        assert!(!inv.is_empty());
    }

    #[test]
    fn broken_vhosts_point_to_missing_containers() {
        let mut inv = Inventory::new();
        inv.add_vhost(VHost::to_container("site.com", "fantasma", 80));
        inv.add_vhost(VHost::to_address("static.com", "1.2.3.4:80"));
        let broken: Vec<_> = inv.broken_vhosts().iter().map(|v| v.domain.clone()).collect();
        assert_eq!(broken, vec!["site.com"]);
    }

    #[test]
    fn vhost_with_present_container_is_not_broken() {
        let mut inv = Inventory::new();
        inv.add_container(Container::new("web", "nginx:1.27"));
        inv.add_vhost(VHost::to_container("site.com", "web", 80));
        assert!(inv.broken_vhosts().is_empty());
    }

    #[test]
    fn iteration_is_ordered_by_name() {
        let mut inv = Inventory::new();
        inv.add_container(Container::new("zeta", "img"));
        inv.add_container(Container::new("alfa", "img"));
        let names: Vec<_> = inv.containers().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["alfa", "zeta"]);
    }
}
