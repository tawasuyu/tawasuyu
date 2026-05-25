//! Device registry: mantiene el índice de dispositivos del kernel presentes,
//! traduce uevents en cambios de `Capability::Device { class }`.

use super::EnteGraph;
use crate::events::GraphEvent;
use arje_card::{Capability, DeviceClass};
use arje_kernel::{UAction, UEvent};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

impl EnteGraph {
    pub async fn on_uevent(&mut self, evt: UEvent, _tx: &mpsc::Sender<GraphEvent>) {
        let class = match &evt.device_class {
            Some(c) => c.clone(),
            None => return, // subsystems sin DeviceClass mapeada — ignoramos.
        };
        match evt.action {
            UAction::Add | UAction::Bind | UAction::Online => {
                let was_first = self.devices_of_class(&class) == 0;
                self.devices.insert(evt.devpath.clone(), evt.clone());
                if was_first {
                    // Primera instancia de la clase → la registramos como
                    // capacidad disponible. El "proveedor" virtual es el
                    // Ente #0 (kernel surface).
                    let cap = Capability::Device { class: class.clone() };
                    self.providers.entry(cap).or_default().insert(self.seed.id);
                    info!(?class, devpath = %evt.devpath, "device capability disponible");
                }
            }
            UAction::Remove | UAction::Unbind | UAction::Offline => {
                self.devices.remove(&evt.devpath);
                if self.devices_of_class(&class) == 0 {
                    let cap = Capability::Device { class: class.clone() };
                    if let Some(set) = self.providers.get_mut(&cap) {
                        set.remove(&self.seed.id);
                    }
                    let revoked: Vec<u64> = self.grants.iter()
                        .filter(|(_, g)| g.cap == cap)
                        .map(|(t, _)| *t)
                        .collect();
                    for t in revoked {
                        self.grants.remove(&t);
                    }
                    warn!(?class, "última instancia removida — capacidad revocada");
                }
            }
            UAction::Change | UAction::Move => {
                self.devices.insert(evt.devpath.clone(), evt);
                debug!(?class, "device modified");
            }
            UAction::Unknown => {}
        }
    }

    fn devices_of_class(&self, class: &DeviceClass) -> usize {
        self.devices.values()
            .filter(|e| e.device_class.as_ref() == Some(class))
            .count()
    }
}
