//! `matilda-core` — el modelo de dominio de administración de servidores.
//!
//! matilda administra servidores, sus contenedores Docker y los hosts
//! virtuales de proxy inverso. Este crate es la parte declarativa y
//! pura: describe *qué* debe existir, sin tocar Docker, SSH ni archivos.
//!
//! - [`host`] — [`Host`], un servidor administrado.
//! - [`container`] — [`Container`], la spec declarativa de un contenedor.
//! - [`vhost`] — [`VHost`], un host virtual de proxy inverso.
//! - [`inventory`] — [`Inventory`], el estado declarado completo.
//!
//! El renderizado de configuración vive en `matilda-config`; la
//! reconciliación deseado-vs-actual, en `matilda-plan`; el transporte
//! (SSH «Linker», agente «Ghost»), en capas superiores.

#![forbid(unsafe_code)]

pub mod container;
pub mod host;
pub mod inventory;
pub mod vhost;

pub use container::{Container, PortMap, RestartPolicy};
pub use host::Host;
pub use inventory::Inventory;
pub use vhost::{Upstream, VHost};
