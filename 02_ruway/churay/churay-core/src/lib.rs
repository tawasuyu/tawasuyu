//! `churay-core` — el motor del instalador/actualizador gráfico de la suite
//! tawasuyu («churay» = *poner/instalar*, quechua).
//!
//! Frontend-agnóstico: `churay-llimphi` (la GUI tipo Office) y un eventual CLI
//! se montan encima. Lo que vive acá:
//!
//! * [`catalog`] — la lista de unidades instalables, tomada de la **única**
//!   tabla de apps del repo (`app-bus`) + componentes de sistema (`arje`).
//! * [`manifest`] — el índice firmado (CAS BLAKE3 + ed25519 vía `agora`).
//!   Modelo compatible con hammer para converger bajo el CAS unificado.
//! * [`install`] — instalación atómica a un `prefix`, desde **bundle**
//!   precompilado (lado A) o **compilando** del workspace (lado B), en modo
//!   **sistema** (root, incluye `arje`) o **local** (`~/.local`, sólo apps).
//! * [`state`] — el registro de lo instalado, que habilita el actualizador.
//! * [`update`] — comparar lo instalado contra un manifiesto: qué hay nuevo o
//!   con actualización.
//! * [`hash`] — `ArtifactHash` (`b3:…`), vendorizado de `hammer-core`.

pub mod catalog;
pub mod hash;
pub mod install;
pub mod manifest;
pub mod repo;
pub mod state;
pub mod update;

pub use catalog::{local_manifest, suite_catalog, SUITE_VERSION};
pub use hash::ArtifactHash;
pub use install::{install_unit, uninstall_unit, InstallConfig, InstallError, InstallMode, Step};
pub use manifest::{Manifest, Scope, SignedManifest, Unit, VerifyError};
pub use repo::{fetch_signed_manifest, CurlFetcher, Fetcher, LocalFetcher, RemoteRepo, RepoError};
pub use state::{InstalledState, InstalledUnit};
pub use update::{check_updates, pending_updates, UpdateInfo, UpdateKind};
