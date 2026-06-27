//! Fuente de capacidades de **sandokan** (el plano de control de procesos).
//!
//! Espeja los subcomandos de `sandokan-cli` (`run`/`list`/`status`/`telemetry`/
//! `stop`) — el contrato es `sandokan_core::Engine`. Declarativa: cada capacidad
//! lleva su programa (`sandokan-cli`) y verbo; el catálogo arma el plan.
//!
//! Igual que mirada, hoy el vocabulario se autoría como datos; la fuente de
//! verdad sigue siendo `sandokan-core`.

use crate::{Capacidad, FuenteCapacidades, Param, Peligro, Superficie, TipoParam};

/// La fuente de sandokan. Sin estado.
pub struct FuenteSandokan;

fn id_card(nombre: &str, desc: &str) -> Param {
    Param { nombre: nombre.into(), tipo: TipoParam::IdCard, requerido: true, descripcion: desc.into() }
}

impl FuenteCapacidades for FuenteSandokan {
    fn superficie(&self) -> Superficie {
        Superficie::Sandokan
    }

    fn capacidades(&self) -> Vec<Capacidad> {
        use Peligro::*;
        let cap = |sufijo, resumen, peligro, params| Capacidad::cli(Superficie::Sandokan, sufijo, "sandokan-cli", resumen, peligro, params);
        vec![
            cap("run", "Arranca un proceso/servicio supervisado (encarna una Card).", Reversible,
                vec![Param::texto("comando", "Comando a ejecutar (ruta + args).")]),
            cap("list", "Lista las unidades en ejecución (id, estado, uptime).", Seguro, vec![]),
            cap("status", "Estado de ciclo de vida de una unidad por su id.", Seguro,
                vec![id_card("id", "Id (ULID) de la unidad — ver el listado.")]),
            cap("telemetry", "Telemetría de una unidad (CPU, memoria, threads, restarts).", Seguro,
                vec![id_card("id", "Id (ULID) de la unidad.")]),
            cap("stop", "Detiene una unidad por su id (con período de gracia).", Disruptivo,
                vec![id_card("id", "Id (ULID) de la unidad a detener.")]),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AtipayError, Catalogo, Invocacion};

    fn cat() -> Catalogo {
        let mut c = Catalogo::new();
        c.registrar(Box::new(FuenteSandokan));
        c
    }

    #[test]
    fn list_es_seguro_y_sin_args() {
        let p = cat().plan(&Invocacion::nueva("sandokan.list")).unwrap();
        assert_eq!(p.programa, "sandokan-cli");
        assert_eq!(p.args, vec!["list"]);
        assert_eq!(p.peligro, Peligro::Seguro);
    }

    #[test]
    fn stop_requiere_id_y_es_disruptivo() {
        let p = cat().plan(&Invocacion::nueva("sandokan.stop").con("id", "01J9X")).unwrap();
        assert_eq!(p.args, vec!["stop", "01J9X"]);
        assert_eq!(p.peligro, Peligro::Disruptivo);
        let err = cat().plan(&Invocacion::nueva("sandokan.stop")).unwrap_err();
        assert!(matches!(err, AtipayError::FaltaArg { .. }));
    }
}
