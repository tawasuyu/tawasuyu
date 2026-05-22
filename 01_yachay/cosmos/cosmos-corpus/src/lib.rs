//! `cosmobiologia-corpus` — la biblioteca de interpretación, indexada.
//!
//! El corpus **no calcula nada** y **no es un set de reglas
//! matemáticas**. Las reglas —qué planeta en qué signo, qué aspecto—
//! las computa el motor astronómico (`cosmobiologia-engine`). El corpus
//! es la **evidencia textual**: fragmentos de los libros —y de la
//! escritura del propio astrólogo— recortados y etiquetados con la
//! combinación exacta que describen. En runtime, las combinaciones de
//! una carta hacen un JOIN contra el corpus y traen los textos —
//! citados, con fuente, sin que ninguna IA invente una palabra.
//!
//! ## Estructura — con TIPOS, porque la astrología tiene gramática
//!
//! Un planeta es una FUNCIÓN; un signo, un ESTILO; una casa, un
//! DOMINIO; un aspecto, una RELACIÓN. No son vectores intercambiables
//! de un mismo espacio plano — colapsarlos a uno solo destruye el
//! significado. El corpus respeta esa gramática:
//!
//! 1. **Arquetipos** ([`Arquetipo`]) — los bloques: cada planeta /
//!    signo / casa / aspecto, con su [`PerfilSemantico`] (dimensiones
//!    psicológicas con peso). Es la ontología que el astrólogo escribe.
//! 2. **Pasajes** ([`Pasaje`]) — el corpus propiamente dicho: texto
//!    real etiquetado por [`CombinacionId`], con su fuente. La
//!    evidencia citable.
//! 3. **Composición** — deducir el perfil de una combinación NO leída a
//!    partir de los bloques. Es un problema de diseño **abierto**: un
//!    producto Hadamard ingenuo da resultados falsos (la dimensión que
//!    un bloque tiene en 0 se queda en 0, no «se enciende»). Este crate
//!    trae las capas 1-2 y deja la 3 sin resolver a propósito.
//!
//! La síntesis narrativa y la separación por dominios vivenciales se
//! resuelven en capas superiores; este crate sólo modela el almacén y
//! el JOIN.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Perfil semántico: dimensiones psicológicas/vivenciales con un peso,
/// por convención en `[-1.0, 1.0]`. Los **nombres** de las dimensiones
/// los define el astrólogo en los datos — el esquema NO los fija (no
/// presupone "Acción", "Estructura", …: el modelo es decisión del
/// astrólogo, no del código).
pub type PerfilSemantico = BTreeMap<String, f32>;

/// El rol gramatical de un arquetipo. No es decorativo: marca que
/// planeta y signo NO son la misma clase de cosa, y por eso no se
/// combinan con un operador único e indiferenciado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoArquetipo {
    /// Una función psíquica (Marte = impulso, Mercurio = cognición…).
    Planeta,
    /// Un estilo o modo (el signo colorea CÓMO se expresa la función).
    Signo,
    /// Un dominio o arena de la vida (la casa dice DÓNDE opera).
    Casa,
    /// Una relación entre dos funciones (conjunción, cuadratura…).
    Aspecto,
}

/// Un bloque constructor: un planeta, signo, casa o aspecto, con el
/// perfil semántico que el astrólogo le asigna.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Arquetipo {
    /// Identificador estable — `"mars"`, `"virgo"`, `"conjunction"`…
    pub nombre: String,
    pub tipo: TipoArquetipo,
    pub perfil: PerfilSemantico,
}

/// El plano vivencial donde una configuración descarga su energía. La
/// contradicción «hiperdisciplinado vs. disperso» no se promedia: cada
/// fuerza vive intacta en su dominio (general en la oficina, poeta
/// disperso en la soledad).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dominio {
    /// Cuerpo, salud, acción directa (casas 1/5/9).
    Vital,
    /// Trabajo, vínculos, entorno (casas 3/7/11).
    Social,
    /// Inconsciente, miedos, indagación interna (casas 4/8/12).
    Psiquico,
}

impl Dominio {
    /// Dominio vivencial de una casa `1..=12`.
    pub fn de_casa(casa: u8) -> Option<Dominio> {
        match casa {
            1 | 5 | 9 => Some(Dominio::Vital),
            3 | 7 | 11 => Some(Dominio::Social),
            4 | 8 | 12 => Some(Dominio::Psiquico),
            2 | 6 | 10 => Some(Dominio::Social), // casas de recursos/trabajo
            _ => None,
        }
    }
}

/// La «etiqueta de código de barras» de una combinación astrológica —
/// la clave del JOIN. Respeta la gramática: cada variante es un tipo
/// distinto de combinación, no una bolsa plana.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CombinacionId {
    /// Un planeta en un signo — `mars·virgo`.
    PlanetaSigno { planeta: String, signo: String },
    /// Un planeta en una casa — `mars@c6`.
    PlanetaCasa { planeta: String, casa: u8 },
    /// Un aspecto entre dos planetas — `mars□saturn`. Los dos extremos
    /// se guardan ORDENADOS, así `mars□saturn` y `saturn□mars` son la
    /// misma clave.
    Aspecto { a: String, kind: String, b: String },
}

impl CombinacionId {
    pub fn planeta_signo(planeta: impl Into<String>, signo: impl Into<String>) -> Self {
        CombinacionId::PlanetaSigno {
            planeta: planeta.into(),
            signo: signo.into(),
        }
    }

    pub fn planeta_casa(planeta: impl Into<String>, casa: u8) -> Self {
        CombinacionId::PlanetaCasa {
            planeta: planeta.into(),
            casa,
        }
    }

    /// Construye un aspecto NORMALIZANDO el orden de los extremos, para
    /// que la dirección no genere dos claves distintas.
    pub fn aspecto(
        a: impl Into<String>,
        kind: impl Into<String>,
        b: impl Into<String>,
    ) -> Self {
        let (a, b) = (a.into(), b.into());
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        CombinacionId::Aspecto {
            a,
            kind: kind.into(),
            b,
        }
    }
}

impl std::fmt::Display for CombinacionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CombinacionId::PlanetaSigno { planeta, signo } => {
                write!(f, "{planeta}·{signo}")
            }
            CombinacionId::PlanetaCasa { planeta, casa } => write!(f, "{planeta}@c{casa}"),
            CombinacionId::Aspecto { a, kind, b } => write!(f, "{a} {kind} {b}"),
        }
    }
}

/// Un fragmento de interpretación: el texto de un autor (o del propio
/// astrólogo) recortado y etiquetado con la combinación que describe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pasaje {
    /// La combinación que este pasaje interpreta — la clave del JOIN.
    pub combinacion: CombinacionId,
    /// El texto, citado literalmente.
    pub texto: String,
    /// Procedencia — autor y obra, o `"propio"`. Convención: un pasaje
    /// con fuente `"deducido"` es un perfil compuesto, no un texto de
    /// libro (capa de composición, aún sin construir).
    pub fuente: String,
    /// Firma semántica del pasaje. Opcional: vacío hasta que se calcule.
    #[serde(default)]
    pub perfil: PerfilSemantico,
    /// Dominio vivencial donde aplica, si el pasaje lo acota.
    #[serde(default)]
    pub dominio: Option<Dominio>,
}

/// El corpus completo: la ontología de arquetipos + los pasajes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Corpus {
    pub arquetipos: Vec<Arquetipo>,
    pub pasajes: Vec<Pasaje>,
}

impl Corpus {
    /// Carga un corpus desde su forma RON (el formato de los archivos
    /// que el astrólogo escribe a mano).
    pub fn desde_ron(texto: &str) -> Result<Corpus, String> {
        ron::from_str(texto).map_err(|e| format!("corpus :: RON inválido: {e}"))
    }

    /// Serializa el corpus a RON.
    pub fn a_ron(&self) -> Result<String, String> {
        ron::to_string(self).map_err(|e| format!("corpus :: no se pudo serializar: {e}"))
    }

    /// El arquetipo con ese nombre y tipo, si existe.
    pub fn arquetipo(&self, nombre: &str, tipo: TipoArquetipo) -> Option<&Arquetipo> {
        self.arquetipos
            .iter()
            .find(|a| a.nombre == nombre && a.tipo == tipo)
    }

    /// Todos los pasajes que interpretan una combinación dada.
    pub fn pasajes_de(&self, id: &CombinacionId) -> Vec<&Pasaje> {
        self.pasajes.iter().filter(|p| &p.combinacion == id).collect()
    }

    /// El JOIN: dada la lista de combinaciones de una carta, devuelve
    /// todos los pasajes del corpus que las interpretan. Cobertura
    /// total — no se salta una combinación que tenga texto. Combinar
    /// estos pasajes en una narrativa coherente (síntesis) es trabajo
    /// de una capa superior; aquí sólo se RECUPERA la evidencia.
    pub fn interpretar(&self, combinaciones: &[CombinacionId]) -> Vec<&Pasaje> {
        let mut out = Vec::new();
        for id in combinaciones {
            out.extend(self.pasajes_de(id));
        }
        out
    }

    /// Combinaciones del corpus que NO tienen ni un solo pasaje — los
    /// huecos que habría que escribir, o cubrir con composición.
    pub fn huecos(&self, combinaciones: &[CombinacionId]) -> Vec<CombinacionId> {
        combinaciones
            .iter()
            .filter(|id| self.pasajes_de(id).is_empty())
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspecto_normaliza_el_orden_de_los_extremos() {
        let ab = CombinacionId::aspecto("mars", "square", "saturn");
        let ba = CombinacionId::aspecto("saturn", "square", "mars");
        assert_eq!(ab, ba, "mars□saturn y saturn□mars son la misma clave");
    }

    #[test]
    fn display_da_un_codigo_de_barras_legible() {
        assert_eq!(
            CombinacionId::planeta_signo("mars", "virgo").to_string(),
            "mars·virgo"
        );
        assert_eq!(
            CombinacionId::planeta_casa("mars", 6).to_string(),
            "mars@c6"
        );
    }

    fn pasaje(id: CombinacionId, texto: &str) -> Pasaje {
        Pasaje {
            combinacion: id,
            texto: texto.into(),
            fuente: "test".into(),
            perfil: PerfilSemantico::new(),
            dominio: None,
        }
    }

    #[test]
    fn interpretar_hace_el_join_de_las_combinaciones() {
        let corpus = Corpus {
            arquetipos: Vec::new(),
            pasajes: vec![
                pasaje(
                    CombinacionId::planeta_signo("mars", "virgo"),
                    "el guerrero cirujano",
                ),
                pasaje(
                    CombinacionId::aspecto("mars", "square", "saturn"),
                    "acción frenada",
                ),
                pasaje(
                    CombinacionId::planeta_signo("moon", "pisces"),
                    "sensibilidad difusa",
                ),
            ],
        };
        // Una carta con sólo dos de las tres combinaciones.
        let carta = [
            CombinacionId::planeta_signo("mars", "virgo"),
            // El orden inverso debe resolver igual.
            CombinacionId::aspecto("saturn", "square", "mars"),
        ];
        let recuperados = corpus.interpretar(&carta);
        assert_eq!(recuperados.len(), 2);
        assert!(recuperados.iter().any(|p| p.texto == "el guerrero cirujano"));
        assert!(recuperados.iter().any(|p| p.texto == "acción frenada"));
    }

    #[test]
    fn huecos_detecta_combinaciones_sin_pasaje() {
        let corpus = Corpus {
            arquetipos: Vec::new(),
            pasajes: vec![pasaje(
                CombinacionId::planeta_signo("mars", "virgo"),
                "x",
            )],
        };
        let carta = [
            CombinacionId::planeta_signo("mars", "virgo"),
            CombinacionId::planeta_signo("venus", "leo"),
        ];
        let huecos = corpus.huecos(&carta);
        assert_eq!(huecos.len(), 1);
        assert_eq!(huecos[0], CombinacionId::planeta_signo("venus", "leo"));
    }

    #[test]
    fn corpus_roundtrip_ron() {
        let corpus = Corpus {
            arquetipos: vec![Arquetipo {
                nombre: "mars".into(),
                tipo: TipoArquetipo::Planeta,
                perfil: BTreeMap::from([("accion".into(), 0.9_f32)]),
            }],
            pasajes: vec![pasaje(
                CombinacionId::planeta_signo("mars", "virgo"),
                "el guerrero cirujano",
            )],
        };
        let ron = corpus.a_ron().expect("serializa");
        let vuelta = Corpus::desde_ron(&ron).expect("deserializa");
        assert_eq!(vuelta.arquetipos.len(), 1);
        assert_eq!(vuelta.pasajes.len(), 1);
        assert_eq!(vuelta.pasajes[0].texto, "el guerrero cirujano");
    }

    #[test]
    fn dominio_de_casa_clasifica_los_planos() {
        assert_eq!(Dominio::de_casa(1), Some(Dominio::Vital));
        assert_eq!(Dominio::de_casa(7), Some(Dominio::Social));
        assert_eq!(Dominio::de_casa(12), Some(Dominio::Psiquico));
        assert_eq!(Dominio::de_casa(13), None);
    }
}
