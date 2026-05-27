//! Conceptos — emisores de campo metaprogramables.
//!
//! Un `Concepto` es una **entidad de diseño**, no de código. Lleva una
//! posición, un radio, modificadores por capa (cuánto emite/drena de
//! `materia/psique/poder/oro` por tick a cada celda dentro del radio) y un
//! `BehaviorHack` opcional que captura la acción de los Lemmings que entran
//! a su radio.
//!
//! Para el motor, una "iglesia", un "banco" o una "comuna" no son tipos
//! distintos: son la misma estructura con números diferentes. La iglesia es
//! `mods.psique > 0, mods.materia < 0` con `hack: forced_action = Sincronizar`.
//! El banco es `mods.oro < 0, mods.poder > 0`. La comuna es `mods.materia >
//! 0, mods.degradacion no se toca` (degradacion no es modificable: es
//! cicatriz emergente del extraer).
//!
//! La unidad es **una pieza de datos**: serializable a JSON, generable por
//! cualquier productor externo (un humano en un panel, un script, una IA
//! offline) sin tocar el código del motor.
//!
//! El motor sigue siendo *tonto*: en `dominium-physics` recorre la lista de
//! conceptos y suma los modificadores con un falloff lineal. Cero IA, cero
//! embeddings, cero narrativa. Solo álgebra sobre la grilla.

use serde::{Deserialize, Serialize};

/// Emisión/drenaje por tick en una celda en el centro del radio. En el
/// borde el valor cae linealmente a cero (falloff lineal).
///
/// `degradacion` no se modifica desde un Concepto — es cicatriz emergente
/// del extraer de los Lemmings, no algo que un emisor pueda revertir.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct LayerMods {
    pub materia: f32,
    pub psique: f32,
    pub poder: f32,
    pub oro: f32,
}

/// Condición que dispara un `BehaviorHack` sobre un Lemming que cae dentro
/// del radio del Concepto.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Trigger {
    /// Cualquier Lemming en el radio queda capturado.
    Always,
    /// Solo si la `energia` del Lemming está por debajo del umbral.
    EnergiaBajo(f32),
    /// Solo si la `edad` del Lemming es mayor al umbral.
    EdadSobre(u32),
}

/// Toma de control de la acción de un Lemming durante `duration` ticks.
///
/// Mientras esté capturado (`hack_lock > 0`), el Lemming ejecuta
/// `forced_action` ignorando cualquier transición que el motor le aplicaría
/// (incluida la desesperación → pelear).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BehaviorHack {
    pub trigger: Trigger,
    /// Byte de la acción forzada (0-5; ver [`crate::Action`]).
    pub forced_action: u8,
    /// Ticks que dura el hack desde que se aplica.
    pub duration: u32,
}

/// Influencia psicológica de un Concepto — Fase B.2.
///
/// A diferencia del `BehaviorHack` (que CONGELA acción por N ticks, una
/// metáfora de coerción/captura), la `Persuasion` empuja el `vector_psi`
/// del agente hacia un objetivo cada tick mientras esté dentro del radio,
/// sin tocar su acción. Es la mecánica canónica de **persuasión** /
/// **propaganda**: el agente sigue siendo libre de actuar, pero su
/// psicología deriva.
///
/// El falloff es lineal (1 en el centro, 0 en el borde) — la mismo que
/// usa `LayerMods` sobre la grilla. Así un agente que entra y sale del
/// radio acumula influencia proporcional al tiempo expuesto y a su
/// proximidad al centro.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Persuasion {
    /// Objetivo psicológico hacia el que se empuja el `vector_psi` del
    /// agente. Convención `[ORDEN, MIEDO, CURIOSIDAD, CORRUPTIBILIDAD]`.
    /// Ej. una "iglesia ortodoxa" usaría `[1.0, 0.5, 0.0, 0.0]`.
    pub target_psi: [f32; 4],
    /// Tasa de convergencia por tick a falloff 1.0 (centro del radio).
    /// `psi_nuevo = psi + rate · falloff · (target − psi)`. Rango útil
    /// 0.01..0.10. Valores grandes producen "lavado de cerebro" en pocos
    /// ticks; chicos generan deriva lenta.
    pub rate: f32,
}

/// Un emisor de campo metaprogramable.
///
/// `sprite_id` es opaco al motor: solo viaja del JSON hasta el backend
/// gráfico (que decide qué pintar). El motor no le mira el valor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Concepto {
    /// Nombre legible (ej. `"iglesia"`, `"banco-central"`, `"comuna"`).
    /// Solo informativo: el motor lo ignora.
    pub id: String,
    /// Identificador opaco del sprite que el backend usa para dibujarlo.
    #[serde(default)]
    pub sprite_id: u32,
    pub pos_x: f32,
    pub pos_y: f32,
    /// Radio de influencia en unidades de celda.
    pub radius: f32,
    /// Cuánto emite/drena por tick en el centro (cae linealmente al borde).
    pub mods: LayerMods,
    /// Toma de control opcional. `None` = solo emite campo.
    #[serde(default)]
    pub hack: Option<BehaviorHack>,
    /// Persuasión psicológica opcional (Fase B.2). `None` = el Concepto
    /// sólo emite campo / hackea acción. Cuando está presente, ADEMÁS
    /// empuja el `vector_psi` de los lemmings dentro del radio cada tick.
    /// Es ortogonal al `hack`: un Concepto puede coercer una acción Y
    /// persuadir psi simultáneamente.
    #[serde(default)]
    pub persuasion: Option<Persuasion>,
}

/// Colección lineal. Sin ordenamiento, sin índice espacial: la sim es
/// chica (decenas de conceptos × miles de celdas/Lemmings) y el costo es
/// despreciable. La iteración es determinista por orden de inserción.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Conceptos {
    pub items: Vec<Concepto>,
}

impl Conceptos {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Agrega un concepto al final. Devuelve su índice.
    pub fn add(&mut self, c: Concepto) -> usize {
        self.items.push(c);
        self.items.len() - 1
    }

    /// Elimina por índice con `swap_remove` — O(1), no preserva el orden.
    pub fn remove(&mut self, i: usize) {
        self.items.swap_remove(i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_remove_swap() {
        let mut cs = Conceptos::new();
        let a = cs.add(Concepto {
            id: "a".into(),
            sprite_id: 0,
            pos_x: 1.0,
            pos_y: 1.0,
            radius: 5.0,
            mods: LayerMods::default(),
            hack: None,
            persuasion: None,
        });
        let _b = cs.add(Concepto {
            id: "b".into(),
            sprite_id: 0,
            pos_x: 2.0,
            pos_y: 2.0,
            radius: 5.0,
            mods: LayerMods::default(),
            hack: None,
            persuasion: None,
        });
        assert_eq!((a, cs.len()), (0, 2));
        cs.remove(a);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs.items[0].id, "b");
    }

    #[test]
    fn json_roundtrip_preserves_concepto() {
        let c = Concepto {
            id: "iglesia".into(),
            sprite_id: 42,
            pos_x: 10.0,
            pos_y: 10.0,
            radius: 6.0,
            mods: LayerMods { materia: -0.1, psique: 0.8, poder: 0.3, oro: 0.0 },
            hack: Some(BehaviorHack {
                trigger: Trigger::EnergiaBajo(20.0),
                forced_action: 2,
                duration: 50,
            }),
            persuasion: None,
        };
        let s = serde_json::to_string(&c).expect("serializa");
        let back: Concepto = serde_json::from_str(&s).expect("deserializa");
        assert_eq!(c, back);
    }

    #[test]
    fn json_collection_roundtrip() {
        let mut cs = Conceptos::new();
        cs.add(Concepto {
            id: "iglesia".into(),
            sprite_id: 1,
            pos_x: 8.0,
            pos_y: 8.0,
            radius: 5.0,
            mods: LayerMods { psique: 0.5, ..Default::default() },
            hack: None,
            persuasion: None,
        });
        cs.add(Concepto {
            id: "banco".into(),
            sprite_id: 2,
            pos_x: 30.0,
            pos_y: 12.0,
            radius: 4.0,
            mods: LayerMods { oro: -0.2, poder: 0.4, ..Default::default() },
            hack: None,
            persuasion: None,
        });
        let s = serde_json::to_string(&cs).expect("serializa");
        let back: Conceptos = serde_json::from_str(&s).expect("deserializa");
        assert_eq!(cs, back);
    }

    #[test]
    fn default_optional_fields_in_json() {
        // sprite_id y hack tienen serde(default); deben aceptar JSONs minimalistas.
        let raw = r#"{
            "id": "minimal",
            "pos_x": 0.0,
            "pos_y": 0.0,
            "radius": 1.0,
            "mods": { "materia": 0.0, "psique": 0.0, "poder": 0.0, "oro": 0.0 }
        }"#;
        let c: Concepto = serde_json::from_str(raw).expect("deserializa minimal");
        assert_eq!(c.sprite_id, 0);
        assert!(c.hack.is_none());
    }
}
