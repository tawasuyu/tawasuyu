//! Fuente de capacidades de **mirada** (el compositor / escritorio).
//!
//! Espeja el vocabulario de `mirada-ctl actions` (la fuente de verdad es
//! `mirada_brain::DesktopAction`) y traduce cada invocación a un plan
//! `mirada-ctl <verbo> [valor]`. `mirada-ctl` une sus args con `:` para formar
//! la acción canónica (`focus-window 5` → `focus-window:5`), así que el plan
//! sólo apila `[verbo, valor]`.
//!
//! Nota de rebanada: hoy las capacidades se autorían como datos acá; un paso
//! futuro es derivarlas de `DesktopAction` directamente (un `catalog()` en
//! `mirada-brain`) para tener una sola fuente de verdad sin triplicar.

use crate::{AtipayError, Capacidad, FuenteCapacidades, Invocacion, Param, Peligro, Plan, Superficie, TipoParam};

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
        let cap = |sufijo, resumen, peligro, params| Capacidad::nueva(Superficie::Mirada, sufijo, resumen, peligro, params);
        vec![
            // Foco.
            cap("focus-next", "Mueve el foco a la siguiente ventana.", Seguro, vec![]),
            cap("focus-prev", "Mueve el foco a la ventana anterior.", Seguro, vec![]),
            cap("focus-window", "Enfoca una ventana por su id (ver el listado de ventanas).", Seguro,
                vec![Param { nombre: "id".into(), tipo: TipoParam::IdVentana, requerido: true, descripcion: "Id de la ventana a enfocar.".into() }]),
            // Mover / cerrar.
            cap("move-forward", "Adelanta la ventana enfocada en el teselado.", Reversible, vec![]),
            cap("move-backward", "Atrasa la ventana enfocada en el teselado.", Reversible, vec![]),
            cap("close-focused", "Cierra la ventana enfocada.", Disruptivo, vec![]),
            cap("close-window", "Cierra una ventana por su id.", Disruptivo,
                vec![Param { nombre: "id".into(), tipo: TipoParam::IdVentana, requerido: true, descripcion: "Id de la ventana a cerrar.".into() }]),
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

    fn plan(&self, inv: &Invocacion) -> Result<Plan, AtipayError> {
        // El verbo de mirada-ctl es el sufijo del id (`mirada.workspace` → `workspace`).
        let verbo = inv.id.strip_prefix("mirada.").ok_or_else(|| AtipayError::Desconocida(inv.id.clone()))?;
        // Encontrá la capacidad para conocer sus params y el nivel de peligro.
        let cap = self
            .capacidades()
            .into_iter()
            .find(|c| c.id == inv.id)
            .ok_or_else(|| AtipayError::Desconocida(inv.id.clone()))?;

        let mut args = vec![verbo.to_string()];
        // Apilá el valor de cada parámetro (en orden), validando enteros y enums.
        for p in &cap.params {
            let valor = inv.arg(&p.nombre)?;
            match &p.tipo {
                TipoParam::Entero => {
                    valor.parse::<i64>().map_err(|_| AtipayError::ArgInvalido {
                        id: inv.id.clone(),
                        arg: p.nombre.clone(),
                        motivo: format!("esperaba un entero, vino '{valor}'"),
                    })?;
                }
                TipoParam::Enum(opciones) => {
                    if !opciones.iter().any(|o| o == valor) {
                        return Err(AtipayError::ArgInvalido {
                            id: inv.id.clone(),
                            arg: p.nombre.clone(),
                            motivo: format!("'{valor}' no es una opción válida ({})", opciones.join("/")),
                        });
                    }
                }
                _ => {}
            }
            args.push(valor.to_string());
        }

        Ok(Plan { id: inv.id.clone(), programa: "mirada-ctl".into(), args, peligro: cap.peligro })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_sin_params() {
        let p = FuenteMirada.plan(&Invocacion::nueva("mirada.focus-next")).unwrap();
        assert_eq!(p.programa, "mirada-ctl");
        assert_eq!(p.args, vec!["focus-next"]);
    }

    #[test]
    fn plan_con_entero() {
        let p = FuenteMirada.plan(&Invocacion::nueva("mirada.workspace").con("n", "3")).unwrap();
        assert_eq!(p.args, vec!["workspace", "3"]);
    }

    #[test]
    fn entero_invalido_es_error() {
        let err = FuenteMirada.plan(&Invocacion::nueva("mirada.workspace").con("n", "tres")).unwrap_err();
        assert!(matches!(err, AtipayError::ArgInvalido { .. }));
    }

    #[test]
    fn enum_fuera_de_dominio_es_error() {
        let err = FuenteMirada.plan(&Invocacion::nueva("mirada.layout").con("modo", "diagonal")).unwrap_err();
        assert!(matches!(err, AtipayError::ArgInvalido { .. }));
    }

    #[test]
    fn falta_arg_requerido_es_error() {
        let err = FuenteMirada.plan(&Invocacion::nueva("mirada.focus-window")).unwrap_err();
        assert!(matches!(err, AtipayError::FaltaArg { .. }));
    }

    #[test]
    fn cerrar_es_disruptivo() {
        let p = FuenteMirada.plan(&Invocacion::nueva("mirada.close-focused")).unwrap();
        assert_eq!(p.peligro, Peligro::Disruptivo);
    }
}
