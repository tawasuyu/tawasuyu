//! Fuente de capacidades del propio **shell `shuma`**: los builtins que la IA
//! puede invocar para manejar la sesión (macros, grupos, sesiones persistentes,
//! telemetría, búsqueda). A diferencia de las otras superficies, el «programa»
//! no es un CLI externo sino el **token del builtin** (`:macro`, `:spawn`…): la
//! `linea_comando` del plan da `:macro <nombre>`, que shuma despacha desde el
//! input como cualquier builtin. Por eso usa `cli_args` con `args_base` vacío
//! (el verbo ES el programa, no se duplica).
//!
//! No se exponen `:?`/`:hacé` (la IA no se invoca a sí misma) ni los builtins de
//! edición de bajo nivel; sólo los que tienen sentido como «acción» pedible.

use crate::{Capacidad, FuenteCapacidades, Param, Peligro, Superficie};

/// La fuente del shell. Sin estado.
pub struct FuenteShuma;

impl FuenteCapacidades for FuenteShuma {
    fn superficie(&self) -> Superficie {
        Superficie::Shuma
    }

    fn capacidades(&self) -> Vec<Capacidad> {
        use Peligro::*;
        // `programa` = token del builtin; `args_base` vacío (el verbo es el programa).
        let b = |sufijo, builtin, resumen, peligro, params| {
            Capacidad::cli_args(Superficie::Shuma, sufijo, builtin, &[], resumen, peligro, params)
        };
        vec![
            b("macro", ":macro", "Corre una macro guardada por su nombre.", Reversible,
                vec![Param::texto("nombre", "Nombre de la macro a ejecutar.")]),
            b("macros", ":macros", "Lista las macros guardadas.", Seguro, vec![]),
            b("guardar-grupo", ":save", "Guarda el grupo de comandos actual con un nombre (F1..F8).", Reversible,
                vec![Param::texto("nombre", "Nombre del grupo a guardar.")]),
            b("grupos", ":groups", "Lista los grupos de comandos guardados.", Seguro, vec![]),
            b("nueva-sesion", ":spawn", "Abre una nueva sesión/workspace persistente.", Reversible, vec![]),
            b("sesiones", ":sessions", "Lista las sesiones persistentes (tmux-like).", Seguro, vec![]),
            b("adjuntar", ":attach", "Se conecta a una sesión persistente por su número.", Reversible,
                vec![Param::entero("n", "Número de sesión a la que adjuntarse.")]),
            b("cerrar-sesion", ":kill-session", "Cierra una sesión persistente por su número.", Disruptivo,
                vec![Param::entero("n", "Número de sesión a cerrar.")]),
            b("trabajos", ":jobs", "Lista los trabajos en segundo plano.", Seguro, vec![]),
            b("stats", ":stats", "Muestra la telemetría local del shell (uso, frecuencias).", Seguro, vec![]),
            b("buscar", ":buscar", "Busca por significado en el historial de comandos.", Seguro,
                vec![Param::texto("consulta", "Qué buscar (lenguaje natural).")]),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Catalogo, Invocacion};

    fn cat() -> Catalogo {
        let mut c = Catalogo::new();
        c.registrar(Box::new(FuenteShuma));
        c
    }

    #[test]
    fn macro_arma_el_builtin_con_el_nombre() {
        let p = cat().plan(&Invocacion::nueva("shuma.macro").con("nombre", "deploy")).unwrap();
        assert_eq!(p.programa, ":macro");
        assert_eq!(p.args, vec!["deploy"]);
        assert_eq!(p.linea_comando(), ":macro deploy");
    }

    #[test]
    fn sesiones_es_solo_el_builtin() {
        let p = cat().plan(&Invocacion::nueva("shuma.sesiones")).unwrap();
        assert_eq!(p.linea_comando(), ":sessions");
        assert_eq!(p.peligro, Peligro::Seguro);
    }

    #[test]
    fn cerrar_sesion_es_disruptivo() {
        let p = cat().plan(&Invocacion::nueva("shuma.cerrar-sesion").con("n", "2")).unwrap();
        assert_eq!(p.peligro, Peligro::Disruptivo);
        assert_eq!(p.linea_comando(), ":kill-session 2");
    }
}
