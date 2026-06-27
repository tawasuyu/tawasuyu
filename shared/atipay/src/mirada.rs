//! Fuente de capacidades de **mirada** (el compositor / escritorio).
//!
//! Espeja el vocabulario de `mirada-ctl actions` (la fuente de verdad es
//! `mirada_brain::DesktopAction`). Es puramente declarativa: cada capacidad
//! lleva su programa (`mirada-ctl`) y sus args base; el catálogo arma el plan
//! (`mirada-ctl <verbo> [valor]`). `mirada-ctl` une sus args con `:` para formar
//! la acción canónica (`focus-window 5` → `focus-window:5`).
//!
//! Nota de rebanada: hoy se autoría como datos; un paso futuro es derivarlo de
//! `DesktopAction` (un `catalog()` en `mirada-brain`) para una sola fuente de
//! verdad sin triplicar.

use crate::{Capacidad, FuenteCapacidades, Param, Peligro, Superficie, TipoParam};

/// La fuente de mirada. Sin estado: el vocabulario es estático.
pub struct FuenteMirada;

/// Modos de teselado válidos para `layout <modo>`.
const LAYOUTS: &[&str] = &["master-stack", "centered-master", "spiral", "grid", "columns", "rows", "monocle"];

impl FuenteCapacidades for FuenteMirada {
    fn superficie(&self) -> Superficie {
        Superficie::Mirada
    }

    fn capacidades(&self) -> Vec<Capacidad> {
        use Peligro::*;
        // Verbo = sufijo del id = verbo de mirada-ctl (el caso común: `cli`).
        let cap = |sufijo, resumen, peligro, params| Capacidad::cli(Superficie::Mirada, sufijo, "mirada-ctl", resumen, peligro, params);
        let id_win = |nombre: &str, desc: &str| Param { nombre: nombre.into(), tipo: TipoParam::IdVentana, requerido: true, descripcion: desc.into() };
        vec![
            // Foco.
            cap("focus-next", "Mueve el foco a la siguiente ventana.", Seguro, vec![]),
            cap("focus-prev", "Mueve el foco a la ventana anterior.", Seguro, vec![]),
            cap("focus-window", "Enfoca una ventana por su id (ver el listado de ventanas).", Seguro,
                vec![id_win("id", "Id de la ventana a enfocar.")]),
            // Mover / cerrar.
            cap("move-forward", "Adelanta la ventana enfocada en el teselado.", Reversible, vec![]),
            cap("move-backward", "Atrasa la ventana enfocada en el teselado.", Reversible, vec![]),
            cap("close-focused", "Cierra la ventana enfocada.", Disruptivo, vec![]),
            cap("close-window", "Cierra una ventana por su id.", Disruptivo,
                vec![id_win("id", "Id de la ventana a cerrar.")]),
            // Estado de ventana.
            cap("toggle-float", "Alterna flotante / teselada la ventana enfocada.", Reversible, vec![]),
            cap("toggle-fullscreen", "Alterna pantalla completa en la ventana enfocada.", Reversible, vec![]),
            cap("toggle-maximize", "Alterna maximizada la ventana enfocada.", Reversible, vec![]),
            // Teselado.
            cap("cycle-layout", "Pasa al siguiente modo de teselado.", Reversible, vec![]),
            cap("layout", "Fija el modo de teselado.", Reversible,
                vec![Param { nombre: "modo".into(), tipo: TipoParam::Enum(LAYOUTS.iter().map(|s| s.to_string()).collect()), requerido: true, descripcion: "Modo de teselado.".into() }]),
            // Escritorios.
            cap("workspace", "Activa un escritorio virtual (1..9).", Reversible,
                vec![Param::entero("n", "Número de escritorio (1..9).")]),
            cap("send-to-workspace", "Manda la ventana enfocada a un escritorio (y la sigue).", Reversible,
                vec![Param::entero("n", "Número de escritorio destino.")]),
            cap("workspace-next", "Va al escritorio siguiente.", Reversible, vec![]),
            cap("workspace-prev", "Va al escritorio anterior.", Reversible, vec![]),
            // Monitores.
            cap("focus-output-next", "Pasa el foco al siguiente monitor.", Seguro, vec![]),
            // Sesión.
            cap("lock", "Bloquea la sesión.", Reversible, vec![]),
            cap("logout", "Cierra la sesión (relevo del compositor).", Disruptivo, vec![]),
            cap("quit", "Apaga el compositor.", Disruptivo, vec![]),
            // Lanzar.
            cap("spawn", "Lanza un comando como cliente Wayland.", Reversible,
                vec![Param::texto("comando", "Comando a ejecutar (entre comillas si lleva espacios).")]),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AtipayError, Catalogo, Invocacion};

    fn cat() -> Catalogo {
        let mut c = Catalogo::new();
        c.registrar(Box::new(FuenteMirada));
        c
    }

    #[test]
    fn plan_sin_params() {
        let p = cat().plan(&Invocacion::nueva("mirada.focus-next")).unwrap();
        assert_eq!(p.programa, "mirada-ctl");
        assert_eq!(p.args, vec!["focus-next"]);
    }

    #[test]
    fn plan_con_entero() {
        let p = cat().plan(&Invocacion::nueva("mirada.workspace").con("n", "3")).unwrap();
        assert_eq!(p.args, vec!["workspace", "3"]);
    }

    #[test]
    fn entero_invalido_es_error() {
        let err = cat().plan(&Invocacion::nueva("mirada.workspace").con("n", "tres")).unwrap_err();
        assert!(matches!(err, AtipayError::ArgInvalido { .. }));
    }

    #[test]
    fn enum_fuera_de_dominio_es_error() {
        let err = cat().plan(&Invocacion::nueva("mirada.layout").con("modo", "diagonal")).unwrap_err();
        assert!(matches!(err, AtipayError::ArgInvalido { .. }));
    }

    #[test]
    fn falta_arg_requerido_es_error() {
        let err = cat().plan(&Invocacion::nueva("mirada.focus-window")).unwrap_err();
        assert!(matches!(err, AtipayError::FaltaArg { .. }));
    }

    #[test]
    fn cerrar_es_disruptivo() {
        let p = cat().plan(&Invocacion::nueva("mirada.close-focused")).unwrap();
        assert_eq!(p.peligro, Peligro::Disruptivo);
    }
}
