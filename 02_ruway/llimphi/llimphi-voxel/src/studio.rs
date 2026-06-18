//! `studio` — el **modelo-documento del creador de mundos**: un [`Project`]
//! agnóstico de UI que junta los *artefactos* (mundos, personajes, escenas) bajo
//! nombre, para que **una interfaz** los cree/edite y la **IA** los emita o lea.
//!
//! Es contenido puro y **serializable** (RON para edición a mano / salida de la IA;
//! postcard para la CAS): no toca GPU ni ventana. La studio app (o cualquier otra)
//! lo carga, lo pinta con sus widgets y lo guarda. Cada artefacto referencia tipos
//! que ya existen en este crate ([`WorldRecipe`], [`Age`]) — el `Project` sólo les
//! pone nombre y los agrupa.

use serde::{Deserialize, Serialize};

use crate::actor::{Actor, Age};
use crate::worldgen::WorldRecipe;
use llimphi_3d::glam::Vec3;

/// Dimensión por defecto de la grilla con la que el editor previsualiza un mundo
/// (cúbica en XZ, alto = 0.4·lado, mínimo 48) — el mismo criterio que la app.
pub const PREVIEW_DIM_XZ: u32 = 128;

/// Calcula el `dim` `[x, y, z]` de un mundo de lado `xz` (alto derivado).
pub fn world_dim(xz: u32) -> [u32; 3] {
    let dy = (xz * 4 / 10).max(48);
    [xz, dy, xz]
}

/// Un **mundo nombrado** del proyecto: nombre + su [`WorldRecipe`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedWorld {
    pub name: String,
    pub recipe: WorldRecipe,
}

impl NamedWorld {
    pub fn new(name: impl Into<String>, recipe: WorldRecipe) -> Self {
        Self { name: name.into(), recipe }
    }
}

/// **Especificación serializable de un personaje**: lo que un editor/IA fija (edad
/// + colores). Se materializa con [`to_actor`](Self::to_actor) en un [`Actor`]
/// posable. Los colores son `[r, g, b]` en `[0,1]` (como [`Actor`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharSpec {
    pub name: String,
    pub age: Age,
    pub skin: [f32; 3],
    pub shirt: [f32; 3],
    pub pants: [f32; 3],
}

impl CharSpec {
    /// Un personaje con la paleta por defecto de [`Actor::new`] a la edad dada.
    pub fn new(name: impl Into<String>, age: Age) -> Self {
        let a = Actor::new(Vec3::ZERO, 0.0);
        Self { name: name.into(), age, skin: a.skin, shirt: a.shirt, pants: a.pants }
    }

    /// Materializa el spec en un [`Actor`] parado en `pos` mirando a `facing`.
    pub fn to_actor(&self, pos: Vec3, facing: f32) -> Actor {
        Actor::new(pos, facing)
            .with_age(self.age)
            .with_colors(self.skin, self.shirt, self.pants)
    }
}

/// El **proyecto**: la bolsa de artefactos del creador. Crece por artefacto
/// (mundos hoy; personajes y escenas a medida que el editor los soporte). Vacío
/// por defecto; [`starter`](Self::starter) trae un par de mundos de arranque.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Project {
    pub worlds: Vec<NamedWorld>,
    #[serde(default)]
    pub characters: Vec<CharSpec>,
}

impl Project {
    /// Proyecto de arranque: el desierto y la pradera (los dos presets), para que
    /// el editor abra con algo que tocar.
    pub fn starter() -> Self {
        Self {
            worlds: vec![
                NamedWorld::new("desierto", WorldRecipe::desert(1337)),
                NamedWorld::new("pradera", WorldRecipe::grassland(1337)),
            ],
            characters: vec![CharSpec::new("personaje", Age::Adult)],
        }
    }

    /// Agrega un mundo y devuelve su índice.
    pub fn add_world(&mut self, w: NamedWorld) -> usize {
        self.worlds.push(w);
        self.worlds.len() - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proyecto_round_trip_ron() {
        let p = Project::starter();
        let s = ron::ser::to_string(&p).expect("serializa a ron");
        let back: Project = ron::from_str(&s).expect("deserializa de ron");
        assert_eq!(back.worlds.len(), p.worlds.len());
        assert_eq!(back.worlds[0].name, "desierto");
        // La receta sobrevive el viaje (un parámetro de muestra).
        assert!((back.worlds[0].recipe.base - p.worlds[0].recipe.base).abs() < 1e-6);
    }

    #[test]
    fn charspec_se_materializa_con_la_edad() {
        let spec = CharSpec::new("nene", Age::Baby);
        let actor = spec.to_actor(Vec3::ZERO, 0.0);
        assert_eq!(actor.age, Age::Baby);
    }

    #[test]
    fn world_dim_minimo_48_de_alto() {
        assert_eq!(world_dim(64)[1], 48); // 64*0.4=25.6 → clamp a 48
        assert_eq!(world_dim(192)[1], 76);
    }
}
