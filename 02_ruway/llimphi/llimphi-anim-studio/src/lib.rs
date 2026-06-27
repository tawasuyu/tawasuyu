//! Biblioteca del studio de animación «rive»: los documentos **serializables**
//! que el editor autora —el grafo de estados ([`doc::Doc`]) y el rig esqueletal
//! ([`rig::RigDoc`])— y el [`Project`] que los junta en el `.ron` que se guarda
//! y se lee.
//!
//! Antes estos tipos vivían dentro del binario del editor; exponerlos como
//! biblioteca permite que **otros consumidores** carguen el mismo formato sin
//! duplicar los structs. El primero es `mirada-fondo`, que reproduce un proyecto
//! como **fondo** (splash/greeter/wallpaper): bakea el rig deformado a frames y
//! los blitea. El editor (`main.rs`) es ahora un frontend más sobre esta lib.

#![forbid(unsafe_code)]

pub mod doc;
pub mod rig;

use serde::{Deserialize, Serialize};

/// El proyecto persistido: ambas superficies juntas en un solo `.ron` (el grafo
/// de estados + el rig esqueletal). Es el formato que escribe el editor y el que
/// carga `mirada-fondo` para reproducir un «rive» como fondo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub doc: doc::Doc,
    #[serde(default = "rig::RigDoc::starter")]
    pub rig: rig::RigDoc,
}

impl Default for Project {
    fn default() -> Self {
        Project {
            doc: doc::Doc::starter(),
            rig: rig::RigDoc::starter(),
        }
    }
}

impl Project {
    /// Carga y parsea un proyecto `.ron` de disco.
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        let p = path.as_ref();
        let s = std::fs::read_to_string(p).map_err(|e| format!("no se pudo leer {p:?}: {e}"))?;
        ron::from_str(&s).map_err(|e| format!("RON inválido en {p:?}: {e}"))
    }
}
