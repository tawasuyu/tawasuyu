//! El actualizador — la pieza que hammer no tiene: comparar lo instalado
//! ([`InstalledState`]) contra un [`Manifest`] y decidir qué cambió.
//!
//! "Actualizar" reusa el mismo camino que "instalar"
//! ([`crate::install::install_unit`]): re-traer el binario nuevo y swap
//! atómico. Acá sólo vive el **diagnóstico** (qué hay para hacer).

use crate::manifest::Manifest;
use crate::state::InstalledState;

/// Qué le pasa a una unidad frente a un manifiesto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateKind {
    /// Instalada y al día (misma versión y mismo hash, si el manifiesto lo trae).
    AlDia,
    /// Instalada pero el manifiesto ofrece una versión/hash distintos.
    Disponible,
    /// En el manifiesto pero no instalada todavía.
    Nueva,
}

/// El veredicto para una unidad concreta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub id: String,
    pub label: String,
    pub kind: UpdateKind,
    /// Versión instalada (si la hay).
    pub installed_version: Option<String>,
    /// Versión que ofrece el manifiesto.
    pub available_version: String,
}

/// Recorre el manifiesto y clasifica cada unidad contra lo instalado. El orden
/// es el del manifiesto (alfabético por label, como sale del catálogo).
pub fn check_updates(state: &InstalledState, manifest: &Manifest) -> Vec<UpdateInfo> {
    manifest
        .units
        .iter()
        .map(|u| {
            let installed = state.get(&u.id);
            let kind = match installed {
                None => UpdateKind::Nueva,
                Some(inst) => {
                    let version_difiere = inst.version != u.version;
                    // Si el manifiesto trae hash, un hash distinto también
                    // cuenta como actualización (re-firma del mismo número).
                    let hash_difiere = u
                        .bin_hash
                        .as_ref()
                        .map(|h| h != &inst.hash)
                        .unwrap_or(false);
                    if version_difiere || hash_difiere {
                        UpdateKind::Disponible
                    } else {
                        UpdateKind::AlDia
                    }
                }
            };
            UpdateInfo {
                id: u.id.clone(),
                label: u.label.clone(),
                kind,
                installed_version: installed.map(|i| i.version.clone()),
                available_version: u.version.clone(),
            }
        })
        .collect()
}

/// Sólo las que tienen algo para hacer (nuevas o con actualización).
pub fn pending_updates(state: &InstalledState, manifest: &Manifest) -> Vec<UpdateInfo> {
    check_updates(state, manifest)
        .into_iter()
        .filter(|u| u.kind != UpdateKind::AlDia)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::ArtifactHash;
    use crate::manifest::{Manifest, Scope, Unit};

    fn unit(id: &str, version: &str) -> Unit {
        Unit {
            id: id.into(),
            label: id.into(),
            version: version.into(),
            category: "ruway".into(),
            icon: "≡".into(),
            description: "x".into(),
            program: id.into(),
            scope: Scope::App,
            suggests: Vec::new(),
            bin_hash: None,
            size_bytes: None,
        }
    }

    #[test]
    fn clasifica_nueva_aldia_y_disponible() {
        let manifest = Manifest::new(
            "2026.07",
            vec![unit("nada", "2026.07"), unit("cosmos", "2026.07"), unit("pluma", "2026.07")],
        );
        let mut state = InstalledState::default();
        // nada al día (misma versión), pluma con versión vieja, cosmos sin instalar.
        state.upsert("nada", "2026.07", ArtifactHash::of_bytes(b"a"));
        state.upsert("pluma", "2026.06", ArtifactHash::of_bytes(b"b"));

        let info = check_updates(&state, &manifest);
        let by = |id: &str| info.iter().find(|u| u.id == id).unwrap().kind;
        assert_eq!(by("nada"), UpdateKind::AlDia);
        assert_eq!(by("pluma"), UpdateKind::Disponible);
        assert_eq!(by("cosmos"), UpdateKind::Nueva);

        let pend = pending_updates(&state, &manifest);
        assert_eq!(pend.len(), 2);
        assert!(pend.iter().all(|u| u.kind != UpdateKind::AlDia));
    }

    #[test]
    fn hash_distinto_con_misma_version_es_actualizacion() {
        let mut u = unit("nada", "2026.07");
        u.bin_hash = Some(ArtifactHash::of_bytes(b"nuevo binario"));
        let manifest = Manifest::new("2026.07", vec![u]);
        let mut state = InstalledState::default();
        state.upsert("nada", "2026.07", ArtifactHash::of_bytes(b"viejo binario"));
        assert_eq!(check_updates(&state, &manifest)[0].kind, UpdateKind::Disponible);
    }
}
