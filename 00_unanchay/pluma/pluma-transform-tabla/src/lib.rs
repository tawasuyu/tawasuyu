//! `pluma-transform-tabla` — ejecutor de `Traducir` por tabla explícita.
//!
//! Recibe un `HashMap<Uuid_atom_madre, String_traducido>` y una lengua
//! destino; produce el cuerpo hija con un `NarrativeAtom` nuevo por cada
//! entrada de la tabla (en el orden de la madre), y la `CartaHebras` 1↔1
//! con origen `Derivado`. Átomos de la madre sin entrada en la tabla se
//! OMITEN en la hija — la asimetría queda explícita: no hay contraparte.
//!
//! El crate no genera texto: la traducción la provee quien sea (humano,
//! LLM externo, traductor automático). Eso desacopla el ejecutor del
//! modelo de generación, y permite mezclar fuentes (algunos párrafos
//! traducidos por LLM, otros por humano, otros omitidos a propósito).

#![forbid(unsafe_code)]

use std::collections::HashMap;

use uuid::Uuid;

use pluma_align::{alinear_explicito, OrigenAlineamiento};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion, Lengua};
use pluma_transform::{
    Ejecutor, ErrorEjecutor, ProductoTransformacion, TipoTransformacion, Transformacion,
};

/// Ejecutor de `TipoTransformacion::Traducir` basado en una tabla
/// pre-poblada de traducciones. Es la forma más simple y más honesta de
/// "traducir" dentro de pluma: nadie miente sobre cómo se hizo el texto;
/// la tabla es el contenido.
pub struct EjecutorTraducirTabla {
    /// `Uuid` del átomo madre → texto ya traducido del párrafo.
    pub tabla: HashMap<Uuid, String>,
    /// Lengua de destino. Se anota en `MetaCuerpo::lengua` del hija y
    /// se valida contra el `TipoTransformacion::Traducir { lengua_destino }`
    /// de la transformación que se le pase: si no coinciden, devuelve
    /// `Backend("lengua_destino no coincide con la del ejecutor")`.
    pub lengua_destino: Lengua,
    /// Sufijo que se concatena al `branch_id` de la madre para nombrar el
    /// branch de la hija. Default: la lengua destino. Para casos donde
    /// el caller quiere algo más descriptivo (`"qu-cuzco"` p.ej.), se
    /// sobrescribe.
    pub branch_suffix: Option<String>,
}

impl EjecutorTraducirTabla {
    /// Constructor mínimo: tabla + lengua. El `branch_suffix` toma la
    /// lengua por defecto.
    pub fn new(tabla: HashMap<Uuid, String>, lengua_destino: impl Into<Lengua>) -> Self {
        Self {
            tabla,
            lengua_destino: lengua_destino.into(),
            branch_suffix: None,
        }
    }

    /// Devuelve `self` con el `branch_suffix` ajustado — útil para
    /// cuerpos derivados con variante regional.
    pub fn con_branch_suffix(mut self, suffix: impl Into<String>) -> Self {
        self.branch_suffix = Some(suffix.into());
        self
    }
}

impl Ejecutor for EjecutorTraducirTabla {
    fn aplicar(
        &self,
        t: &Transformacion,
        madre: &Cuerpo,
        ahora: u64,
    ) -> Result<ProductoTransformacion, ErrorEjecutor> {
        // Validar tipo y compatibilidad de lengua.
        let lengua_esperada = match &t.tipo {
            TipoTransformacion::Traducir { lengua_destino } => lengua_destino,
            _ => return Err(ErrorEjecutor::TipoNoSoportado),
        };
        if lengua_esperada != &self.lengua_destino {
            return Err(ErrorEjecutor::Backend(format!(
                "lengua_destino de la transformación ({}) no coincide con la del ejecutor ({})",
                lengua_esperada, self.lengua_destino
            )));
        }
        // La madre debe contener al menos un átomo cubierto por la tabla.
        if !madre.orden.iter().any(|id| self.tabla.contains_key(id)) {
            return Err(ErrorEjecutor::MadreInvalida(
                "la tabla no cubre ningún átomo de la madre",
            ));
        }

        // Construir el cuerpo hija.
        let suffix = self
            .branch_suffix
            .clone()
            .unwrap_or_else(|| self.lengua_destino.clone());
        let mut hija = Cuerpo::nuevo(
            format!("{}-{}", madre.branch_id, suffix),
            format!("{} ({})", madre.metadatos.nombre_legible, self.lengua_destino),
            Intencion::Traduccion,
            ahora,
        )
        .deriva_de(madre.id, ahora)
        .con_lengua(self.lengua_destino.clone());

        let mut atoms_nuevos: Vec<NarrativeAtom> = Vec::with_capacity(madre.orden.len());
        let mut pares: Vec<(Uuid, Uuid, f32)> = Vec::with_capacity(madre.orden.len());

        for &id_madre in &madre.orden {
            let Some(texto) = self.tabla.get(&id_madre) else {
                // Esta posición de la madre queda sin contraparte — bien.
                continue;
            };
            let atom = NarrativeAtom::new(texto.clone(), &hija.branch_id);
            let id_hija = atom.id;
            atoms_nuevos.push(atom);
            hija.agregar(id_hija, ahora);
            pares.push((id_madre, id_hija, 1.0));
        }

        let carta = alinear_explicito(
            madre,
            &hija,
            &pares,
            OrigenAlineamiento::Derivado {
                transformacion: t.id,
                timestamp: ahora,
            },
        );

        Ok(ProductoTransformacion {
            hija,
            atoms_nuevos,
            carta,
        })
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn madre_es(textos: &[&str]) -> (Cuerpo, Vec<NarrativeAtom>) {
        let mut c = Cuerpo::nuevo("es", "es (original)", Intencion::Original, 100);
        let atoms: Vec<NarrativeAtom> =
            textos.iter().map(|t| NarrativeAtom::new(*t, "es")).collect();
        for a in &atoms {
            c.agregar(a.id, 101);
        }
        (c, atoms)
    }

    #[test]
    fn traduce_madre_completa_y_genera_hebras_derivadas() {
        let (madre, atoms_madre) = madre_es(&["uno", "dos", "tres"]);
        let mut tabla = HashMap::new();
        tabla.insert(atoms_madre[0].id, "huk".to_string());
        tabla.insert(atoms_madre[1].id, "iskay".to_string());
        tabla.insert(atoms_madre[2].id, "kimsa".to_string());

        let ejecutor = EjecutorTraducirTabla::new(tabla, "qu");
        let t = Transformacion::nueva(
            madre.id,
            Uuid::new_v4(),
            TipoTransformacion::Traducir {
                lengua_destino: "qu".into(),
            },
            "tester",
            200,
        );
        let prod = ejecutor.aplicar(&t, &madre, 200).unwrap();

        // 3 atoms nuevos, 3 entradas en orden de la madre.
        assert_eq!(prod.atoms_nuevos.len(), 3);
        assert_eq!(prod.hija.orden.len(), 3);
        assert_eq!(prod.atoms_nuevos[0].content.as_str(), "huk");
        assert_eq!(prod.atoms_nuevos[1].content.as_str(), "iskay");
        assert_eq!(prod.atoms_nuevos[2].content.as_str(), "kimsa");

        // Carta 1↔1 con origen Derivado a t.id.
        assert_eq!(prod.carta.hebras.len(), 3);
        for h in &prod.carta.hebras {
            match &h.origen {
                OrigenAlineamiento::Derivado { transformacion, timestamp } => {
                    assert_eq!(*transformacion, t.id);
                    assert_eq!(*timestamp, 200);
                }
                otro => panic!("origen inesperado: {otro:?}"),
            }
            assert_eq!(h.fuerza, 1.0);
        }

        // El cuerpo hija deriva de la madre, está fresco, lengua qu.
        assert_eq!(prod.hija.metadatos.derivado_de, Some(madre.id));
        assert_eq!(prod.hija.metadatos.fresco_hasta, Some(200));
        assert_eq!(prod.hija.metadatos.lengua.as_deref(), Some("qu"));
    }

    #[test]
    fn tabla_con_huecos_omite_atoms_sin_traduccion() {
        let (madre, atoms_madre) = madre_es(&["a", "b", "c"]);
        let mut tabla = HashMap::new();
        // Solo a y c — b queda sin traducción.
        tabla.insert(atoms_madre[0].id, "A".into());
        tabla.insert(atoms_madre[2].id, "C".into());

        let ejecutor = EjecutorTraducirTabla::new(tabla, "qu");
        let t = Transformacion::nueva(
            madre.id,
            Uuid::new_v4(),
            TipoTransformacion::Traducir { lengua_destino: "qu".into() },
            "x",
            10,
        );
        let prod = ejecutor.aplicar(&t, &madre, 10).unwrap();

        assert_eq!(prod.hija.orden.len(), 2);
        assert_eq!(prod.atoms_nuevos[0].content.as_str(), "A");
        assert_eq!(prod.atoms_nuevos[1].content.as_str(), "C");

        // Solo dos hebras: la del medio (b) queda huérfana.
        assert_eq!(prod.carta.hebras.len(), 2);
        let toca_b = prod.carta.hebras.iter().any(|h| h.toca(atoms_madre[1].id));
        assert!(!toca_b, "la hebra no debería tocar al átomo b (sin traducción)");
    }

    #[test]
    fn lengua_destino_inconsistente_devuelve_backend_error() {
        let (madre, atoms_madre) = madre_es(&["a"]);
        let mut tabla = HashMap::new();
        tabla.insert(atoms_madre[0].id, "A".into());

        let ejecutor = EjecutorTraducirTabla::new(tabla, "qu");
        // Transformación pide traducir a en, el ejecutor está configurado a qu.
        let t = Transformacion::nueva(
            madre.id,
            Uuid::new_v4(),
            TipoTransformacion::Traducir { lengua_destino: "en".into() },
            "x",
            1,
        );
        match ejecutor.aplicar(&t, &madre, 1) {
            Err(ErrorEjecutor::Backend(msg)) => assert!(msg.contains("no coincide")),
            otro => panic!("esperaba Backend, fue {otro:?}"),
        }
    }

    #[test]
    fn tipo_no_traducir_devuelve_tipo_no_soportado() {
        let (madre, _) = madre_es(&["a"]);
        let ejecutor = EjecutorTraducirTabla::new(HashMap::new(), "qu");
        let t = Transformacion::nueva(
            madre.id,
            Uuid::new_v4(),
            TipoTransformacion::Identidad,
            "x",
            1,
        );
        assert!(matches!(
            ejecutor.aplicar(&t, &madre, 1),
            Err(ErrorEjecutor::TipoNoSoportado)
        ));
    }

    #[test]
    fn tabla_que_no_cubre_la_madre_es_madre_invalida() {
        let (madre, _atoms_madre) = madre_es(&["a"]);
        // Tabla con un Uuid que NO pertenece a la madre.
        let mut tabla = HashMap::new();
        tabla.insert(Uuid::new_v4(), "A".into());

        let ejecutor = EjecutorTraducirTabla::new(tabla, "qu");
        let t = Transformacion::nueva(
            madre.id,
            Uuid::new_v4(),
            TipoTransformacion::Traducir { lengua_destino: "qu".into() },
            "x",
            1,
        );
        assert!(matches!(
            ejecutor.aplicar(&t, &madre, 1),
            Err(ErrorEjecutor::MadreInvalida(_))
        ));
    }

    #[test]
    fn con_branch_suffix_override_se_aplica_al_hija() {
        let (madre, atoms_madre) = madre_es(&["a"]);
        let mut tabla = HashMap::new();
        tabla.insert(atoms_madre[0].id, "A".into());

        let ejecutor =
            EjecutorTraducirTabla::new(tabla, "qu").con_branch_suffix("qu-cuzco");
        let t = Transformacion::nueva(
            madre.id,
            Uuid::new_v4(),
            TipoTransformacion::Traducir { lengua_destino: "qu".into() },
            "x",
            1,
        );
        let prod = ejecutor.aplicar(&t, &madre, 1).unwrap();
        assert_eq!(prod.hija.branch_id, "es-qu-cuzco");
    }
}
