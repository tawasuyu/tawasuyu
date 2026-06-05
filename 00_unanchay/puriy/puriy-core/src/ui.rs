//! Preferencias de interfaz persistidas — orientación de las pestañas y los
//! **spaces** (pestañas de alto nivel) del usuario.
//!
//! Vive en el [`Profile`](crate::Profile) para sobrevivir entre sesiones. El
//! chrome (`puriy-llimphi`) las lee al arrancar y las reescribe cuando el
//! usuario cambia la orientación o agrega/saca un space. Todo con
//! `#[serde(default)]` para que los perfiles viejos (schema 1, sin este campo)
//! sigan cargando sin migración.

use serde::{Deserialize, Serialize};

/// Un space persistido: su nombre y el glifo de su diente. La membresía de las
/// pestañas y sus URLs no se guardan acá todavía (la restauración completa de
/// sesión es trabajo futuro) — esto fija la *estructura* de spaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpacePref {
    pub name: String,
    pub icon: String,
}

impl SpacePref {
    pub fn new(name: impl Into<String>, icon: impl Into<String>) -> Self {
        Self { name: name.into(), icon: icon.into() }
    }
}

/// Preferencias de UI del navegador.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiPrefs {
    /// `"horizontal"` (barra clásica, un nivel) o `"vertical"` (sidebar de
    /// dientes estilo cosmos). Strings estables — el chrome los mapea a su
    /// enum.
    #[serde(default = "default_orientation")]
    pub orientation: String,
    /// Spaces del usuario, en orden. Siempre se persiste al menos uno.
    #[serde(default = "default_spaces")]
    pub spaces: Vec<SpacePref>,
}

fn default_orientation() -> String {
    "horizontal".into()
}

fn default_spaces() -> Vec<SpacePref> {
    vec![SpacePref::new("Principal", "◆")]
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            orientation: default_orientation(),
            spaces: default_spaces(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_trae_un_space_y_horizontal() {
        let u = UiPrefs::default();
        assert_eq!(u.orientation, "horizontal");
        assert_eq!(u.spaces.len(), 1);
        assert_eq!(u.spaces[0].name, "Principal");
    }

    #[test]
    fn deserializa_sin_campos_con_defaults() {
        // Un Profile viejo (schema 1) no tiene `ui` — al agregarlo con
        // `#[serde(default)]`, un objeto vacío debe rellenar todo.
        let u: UiPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(u, UiPrefs::default());
    }
}
