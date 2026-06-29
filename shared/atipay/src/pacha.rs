//! Fuente de capacidades de **pacha** (contextos de usuario).
//!
//! Espeja los verbos del CLI `pacha` (la fuente de verdad es
//! `pacha-cli`). Le da a la IA de shuma un vocabulario para *manejar el modo
//! de uso* del escritorio: cambiar de contexto, listarlos, cerrarlos. Como el
//! resto de atipay es puramente declarativa — arma el plan (`pacha <verbo>
//! [id]`), no ejecuta nada.

use crate::{Capacidad, FuenteCapacidades, Param, Peligro, Superficie, TipoParam};

/// La fuente de pacha. Sin estado: el vocabulario es estático.
pub struct FuentePacha;

impl FuenteCapacidades for FuentePacha {
    fn superficie(&self) -> Superficie {
        Superficie::Pacha
    }

    fn capacidades(&self) -> Vec<Capacidad> {
        use Peligro::*;
        let cap = |sufijo, resumen, peligro, params| {
            Capacidad::cli(Superficie::Pacha, sufijo, "pacha", resumen, peligro, params)
        };
        let id_pacha = |desc: &str| Param {
            nombre: "id".into(),
            tipo: TipoParam::Texto,
            requerido: true,
            descripcion: desc.into(),
        };
        vec![
            cap(
                "list",
                "Lista los contextos de usuario definidos y cuál está activo.",
                Seguro,
                vec![],
            ),
            cap(
                "switch",
                "Cambia al contexto indicado (aplica su perfil, apps y política de recursos; \
                 al saliente le aplica su default: background, pausa o cerrar).",
                Reversible,
                vec![id_pacha("Id del contexto a activar (ej. 'oficina', 'juegos').")],
            ),
            cap(
                "close",
                "Cierra un contexto (libera sus apps y recursos) sin cambiar el foco.",
                Disruptivo,
                vec![id_pacha("Id del contexto a cerrar.")],
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Catalogo, Invocacion, Peligro};

    fn cat() -> Catalogo {
        let mut c = Catalogo::new();
        c.registrar(Box::new(FuentePacha));
        c
    }

    #[test]
    fn switch_arma_el_plan_con_id() {
        let p = cat().plan(&Invocacion::nueva("pacha.switch").con("id", "juegos")).unwrap();
        assert_eq!(p.programa, "pacha");
        assert_eq!(p.args, vec!["switch", "juegos"]);
        assert_eq!(p.peligro, Peligro::Reversible);
    }

    #[test]
    fn list_sin_params() {
        let p = cat().plan(&Invocacion::nueva("pacha.list")).unwrap();
        assert_eq!(p.args, vec!["list"]);
    }

    #[test]
    fn close_es_disruptivo() {
        let p = cat().plan(&Invocacion::nueva("pacha.close").con("id", "oficina")).unwrap();
        assert_eq!(p.peligro, Peligro::Disruptivo);
        assert_eq!(p.args, vec!["close", "oficina"]);
    }

    #[test]
    fn switch_sin_id_es_error() {
        assert!(cat().plan(&Invocacion::nueva("pacha.switch")).is_err());
    }
}
