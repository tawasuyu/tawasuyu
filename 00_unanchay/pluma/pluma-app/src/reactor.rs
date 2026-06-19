//! Puente entre el modelo de pluma y [`pluma_reactor`].
//!
//! El haz de cuerpos **es una hoja**: cada cuerpo es una **celda** (una
//! columna), y cada [`Transformacion`] madre→hija es una **relación-fórmula**
//! (`hija = FUNC(madre)`). Con eso, cuando un cuerpo cambia podemos saber qué
//! hijas regenerar y **en qué orden** (topológico) — incluso con cadenas
//! (madre → traducción → resumen-de-la-traducción) o relaciones cruzadas.
//!
//! Esto NO ejecuta nada: sólo da el orden. El cómputo real (traducir/resumir
//! por LLM) lo dispara `update` con la maquinaria async que ya existe.

use std::collections::HashMap;

use pluma_reactor::{CellRef, Reactor};
use pluma_transform::{TipoTransformacion, Transformacion};
use uuid::Uuid;

/// Nombre de función yupay para cada tipo de transformación. Sólo importa de
/// cara al futuro (qué transform disparar); para el orden de recálculo basta
/// con que la arista madre→hija exista.
fn funcion_de(t: &TipoTransformacion) -> &'static str {
    match t {
        TipoTransformacion::Identidad => "IDENTIDAD",
        TipoTransformacion::Traducir { .. } => "TRADUCIR",
        TipoTransformacion::Tono { .. } => "TONO",
        TipoTransformacion::Resumir { .. } => "RESUMIR",
        TipoTransformacion::Reescribir { .. } => "REESCRIBIR",
        TipoTransformacion::Custom { .. } => "CUSTOM",
    }
}

/// El reactor del haz + el mapeo cuerpo↔celda para traducir de vuelta.
pub struct ReactorHaz {
    reactor: Reactor,
    por_id: HashMap<Uuid, CellRef>,
    por_celda: HashMap<CellRef, Uuid>,
}

impl ReactorHaz {
    /// Arma el reactor: una columna por cuerpo (en `orden_cuerpos`), una
    /// relación-fórmula por transformación. Las transformaciones que apunten a
    /// cuerpos fuera del orden se ignoran.
    pub fn construir(orden_cuerpos: &[Uuid], transformaciones: &[Transformacion]) -> Self {
        let mut por_id = HashMap::new();
        let mut por_celda = HashMap::new();
        for (col, id) in orden_cuerpos.iter().enumerate() {
            let cell = CellRef::new(col as u32, 0);
            por_id.insert(*id, cell);
            por_celda.insert(cell, *id);
        }
        let mut reactor = Reactor::new();
        for t in transformaciones {
            let (Some(&hija), Some(&madre)) = (por_id.get(&t.hija), por_id.get(&t.madre)) else {
                continue;
            };
            // `madre` se imprime como A1/B1/… (Display de CellRef).
            let src = format!("{}({})", funcion_de(&t.tipo), madre);
            // `set_formula` sólo falla si la fórmula no parsea — acá la armamos
            // nosotros, así que no debería; si pasara, esa arista se omite.
            let _ = reactor.set_formula(hija, &src);
        }
        Self { reactor, por_id, por_celda }
    }

    /// Cuerpos a **regenerar** cuando `cambiado` cambia, en **orden
    /// topológico** (cada hija después de su madre). Vacío si no tiene hijas.
    /// Es lo que el on-blur recorre: por cada uno, dispara su transform si está
    /// *stale*.
    pub fn regenerar_en_orden(&self, cambiado: Uuid) -> Vec<Uuid> {
        let Some(&cell) = self.por_id.get(&cambiado) else {
            return Vec::new();
        };
        self.reactor
            .downstream(cell)
            .into_iter()
            .filter_map(|c| self.por_celda.get(&c).copied())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(madre: Uuid, hija: Uuid, tipo: TipoTransformacion) -> Transformacion {
        Transformacion::nueva(madre, hija, tipo, "test", 100)
    }

    fn traducir() -> TipoTransformacion {
        TipoTransformacion::Traducir { lengua_destino: "en".into() }
    }

    #[test]
    fn una_traduccion_se_regenera_al_cambiar_la_madre() {
        let es = Uuid::from_u128(1);
        let en = Uuid::from_u128(2);
        let orden = vec![es, en];
        let trs = vec![tr(es, en, traducir())];
        let rh = ReactorHaz::construir(&orden, &trs);
        // Editar el español obliga a regenerar el inglés.
        assert_eq!(rh.regenerar_en_orden(es), vec![en]);
        // Editar el inglés no regenera nada (es hoja).
        assert!(rh.regenerar_en_orden(en).is_empty());
    }

    #[test]
    fn cadena_madre_traduccion_resumen_en_orden() {
        let es = Uuid::from_u128(1);
        let en = Uuid::from_u128(2);
        let resumen_en = Uuid::from_u128(3);
        let orden = vec![es, en, resumen_en];
        let trs = vec![
            tr(es, en, traducir()),
            // El resumen deriva de la TRADUCCIÓN, no del original.
            tr(en, resumen_en, TipoTransformacion::Resumir { palabras_objetivo: Some(20) }),
        ];
        let rh = ReactorHaz::construir(&orden, &trs);
        // Cambiar el español cascada: primero el inglés, después su resumen.
        assert_eq!(rh.regenerar_en_orden(es), vec![en, resumen_en]);
        // Cambiar el inglés sólo afecta su resumen.
        assert_eq!(rh.regenerar_en_orden(en), vec![resumen_en]);
    }

    #[test]
    fn una_madre_con_varias_hijas() {
        // 1-n: el original tiene traducción, tono y resumen.
        let es = Uuid::from_u128(1);
        let en = Uuid::from_u128(2);
        let formal = Uuid::from_u128(3);
        let corto = Uuid::from_u128(4);
        let orden = vec![es, en, formal, corto];
        let trs = vec![
            tr(es, en, traducir()),
            tr(es, formal, TipoTransformacion::Tono { etiqueta: "formal".into() }),
            tr(es, corto, TipoTransformacion::Resumir { palabras_objetivo: None }),
        ];
        let rh = ReactorHaz::construir(&orden, &trs);
        let hijas = rh.regenerar_en_orden(es);
        assert_eq!(hijas.len(), 3);
        assert!([en, formal, corto].iter().all(|x| hijas.contains(x)));
    }
}
