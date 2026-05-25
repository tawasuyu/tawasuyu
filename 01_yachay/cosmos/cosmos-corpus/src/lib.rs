//! `cosmos_app-corpus` — la biblioteca de interpretación, indexada.
//!
//! El corpus **no calcula nada** y **no es un set de reglas
//! matemáticas**. Las reglas —qué planeta en qué signo, qué aspecto—
//! las computa el motor astronómico (`cosmos_app-engine`). El corpus
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
//! La **rebanada por dominio** —ver el cuerpo de la carta en tajadas—
//! sí vive aquí ([`rebanar_por_dominio`]): es geometría sobre las
//! claves, no síntesis. La carta es una sola configuración; cortarla
//! por dominio vivencial no la promedia, la MIRA desde un plano. Lo
//! único que queda fuera es la síntesis narrativa —tejer los pasajes
//! recuperados en un texto continuo—, trabajo de una capa superior.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
///
/// Se (de)serializa como una **cadena** legible (`mars·virgo`,
/// `mars@c6`, `mars square saturn`) para que el corpus se escriba a
/// mano sin pelear con la sintaxis de enums. El punto medio `·` admite
/// el alias ASCII `/` (`mars/virgo`), más fácil de teclear.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

impl FromStr for CombinacionId {
    type Err = String;

    /// Parsea el código de barras: `planeta·signo` (o `planeta/signo`),
    /// `planeta@cN`, o `a kind b` (tres tokens separados por espacios).
    fn from_str(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if let Some((planeta, signo)) = s.split_once('·').or_else(|| s.split_once('/')) {
            return Ok(CombinacionId::planeta_signo(planeta.trim(), signo.trim()));
        }
        if let Some((planeta, casa)) = s.split_once("@c") {
            let casa: u8 = casa
                .trim()
                .parse()
                .map_err(|_| format!("casa inválida en '{s}'"))?;
            return Ok(CombinacionId::planeta_casa(planeta.trim(), casa));
        }
        let toks: Vec<&str> = s.split_whitespace().collect();
        if toks.len() == 3 {
            return Ok(CombinacionId::aspecto(toks[0], toks[1], toks[2]));
        }
        Err(format!("combinación no reconocida: '{s}'"))
    }
}

impl Serialize for CombinacionId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for CombinacionId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// La posición de un planeta en una carta concreta: en qué signo y en
/// qué casa cae. Es la materia prima desde la que se derivan las
/// [`CombinacionId`] de la carta — el puente entre lo que el motor
/// astronómico calcula y las claves del JOIN del corpus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Colocacion {
    pub planeta: String,
    pub signo: String,
    pub casa: u8,
}

/// Un aspecto medido en una carta: dos planetas y el ángulo que los une.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AspectoEnCarta {
    pub a: String,
    pub kind: String,
    pub b: String,
}

/// Deriva TODAS las combinaciones de una carta: por cada planeta, su
/// `planeta·signo` y su `planeta@cN`; por cada aspecto medido, su
/// `a kind b`. El resultado es la lista que se le pasa a
/// [`Corpus::interpretar`] para hacer el JOIN.
pub fn combinaciones_de_carta(
    colocaciones: &[Colocacion],
    aspectos: &[AspectoEnCarta],
) -> Vec<CombinacionId> {
    let mut out = Vec::with_capacity(colocaciones.len() * 2 + aspectos.len());
    for c in colocaciones {
        out.push(CombinacionId::planeta_signo(&c.planeta, &c.signo));
        out.push(CombinacionId::planeta_casa(&c.planeta, c.casa));
    }
    for a in aspectos {
        out.push(CombinacionId::aspecto(&a.a, &a.kind, &a.b));
    }
    out
}

/// La **tomografía** de la carta: reparte cada combinación en el dominio
/// —o dominios— vivencial donde descarga su energía.
///
/// La carta es UNA sola configuración; rebanarla por dominio no la
/// promedia ni la mutila, la MIRA desde un plano —como ver un cuerpo en
/// tajadas—. Las reglas del corte:
///
/// - un `planeta@cN` cae en el dominio de su casa;
/// - un `planeta·signo` hereda el dominio de la casa donde ESE planeta
///   está colocado en la carta;
/// - un aspecto **puentea**: aparece en el dominio de cada uno de sus
///   dos extremos. Que una misma combinación salga en dos rebanadas no
///   es un error — es la conexión real entre dos planos.
///
/// Una combinación cuyo planeta no figura en `colocaciones` se omite (no
/// hay forma de saber en qué dominio ubicarla).
pub fn rebanar_por_dominio(
    colocaciones: &[Colocacion],
    combinaciones: &[CombinacionId],
) -> BTreeMap<Dominio, Vec<CombinacionId>> {
    let casa_de: BTreeMap<&str, u8> = colocaciones
        .iter()
        .map(|c| (c.planeta.as_str(), c.casa))
        .collect();
    let dominio_de = |planeta: &str| -> Option<Dominio> {
        casa_de.get(planeta).copied().and_then(Dominio::de_casa)
    };

    let mut tajadas: BTreeMap<Dominio, Vec<CombinacionId>> = BTreeMap::new();
    for id in combinaciones {
        let dominios: Vec<Dominio> = match id {
            CombinacionId::PlanetaCasa { casa, .. } => {
                Dominio::de_casa(*casa).into_iter().collect()
            }
            CombinacionId::PlanetaSigno { planeta, .. } => {
                dominio_de(planeta).into_iter().collect()
            }
            CombinacionId::Aspecto { a, b, .. } => {
                let mut ds = Vec::new();
                for p in [a.as_str(), b.as_str()] {
                    if let Some(d) = dominio_de(p) {
                        if !ds.contains(&d) {
                            ds.push(d);
                        }
                    }
                }
                ds
            }
        };
        for d in dominios {
            tajadas.entry(d).or_default().push(id.clone());
        }
    }
    tajadas
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

/// Evidencia **vecina** de una combinación que no tiene pasaje propio:
/// pasajes del corpus que comparten uno de sus componentes (el planeta,
/// el signo, la casa, o el tipo de aspecto).
///
/// Es la respuesta honesta al problema de la «composición». El corpus
/// **no sintetiza** un texto para una combinación no escrita —eso sería
/// inventar—. Tampoco multiplica perfiles numéricos: el producto
/// Hadamard (y parientes) se descartó porque da falsos (una dimensión
/// en 0 nunca «se enciende») y, sobre todo, porque un perfil compuesto
/// es una conjetura, no evidencia. Lo que sí es honesto: traer las
/// citas reales de contextos parecidos y que el astrólogo componga él.
#[derive(Debug, Clone)]
pub struct EvidenciaVecina<'a> {
    /// Qué componente comparten — `"planeta mars"`, `"signo virgo"`,
    /// `"casa 6"`, `"aspecto square"`.
    pub comparte: String,
    pub pasajes: Vec<&'a Pasaje>,
}

/// `true` si la combinación involucra a ese planeta, en cualquier rol.
fn combinacion_usa_planeta(c: &CombinacionId, planeta: &str) -> bool {
    match c {
        CombinacionId::PlanetaSigno { planeta: p, .. } => p == planeta,
        CombinacionId::PlanetaCasa { planeta: p, .. } => p == planeta,
        CombinacionId::Aspecto { a, b, .. } => a == planeta || b == planeta,
    }
}

/// El corpus completo: la ontología de arquetipos + los pasajes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Corpus {
    pub arquetipos: Vec<Arquetipo>,
    pub pasajes: Vec<Pasaje>,
}

impl Corpus {
    /// Carga un corpus desde su forma RON (el format de los archivos
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

    /// El JOIN **rebanado por dominio**: para cada plano vivencial, los
    /// pasajes que lo interpretan. Es la entrada directa de un gráfico
    /// «por tajadas» — una rebanada, una vista del cuerpo de la carta.
    /// Un aspecto que puentea dos dominios trae sus pasajes a las dos
    /// rebanadas.
    pub fn interpretar_por_dominio(
        &self,
        colocaciones: &[Colocacion],
        aspectos: &[AspectoEnCarta],
    ) -> BTreeMap<Dominio, Vec<&Pasaje>> {
        let combinaciones = combinaciones_de_carta(colocaciones, aspectos);
        rebanar_por_dominio(colocaciones, &combinaciones)
            .into_iter()
            .map(|(dominio, ids)| {
                let mut pasajes = Vec::new();
                for id in &ids {
                    pasajes.extend(self.pasajes_de(id));
                }
                (dominio, pasajes)
            })
            .collect()
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

    /// Pasajes cuya combinación cumple un predicado.
    fn pasajes_donde(&self, pred: impl Fn(&CombinacionId) -> bool) -> Vec<&Pasaje> {
        self.pasajes.iter().filter(|p| pred(&p.combinacion)).collect()
    }

    /// La **capa de composición**, hecha con honestidad: para una
    /// combinación SIN pasaje propio, junta la evidencia vecina —
    /// pasajes que comparten uno de sus componentes—. No sintetiza un
    /// texto ni compone perfiles; son citas reales de contextos
    /// parecidos, agrupadas por el componente que comparten, para que
    /// el astrólogo componga. Si la combinación SÍ tiene pasaje propio,
    /// devuelve vacío — no hace falta. Ver [`EvidenciaVecina`].
    pub fn evidencia_relacionada(&self, id: &CombinacionId) -> Vec<EvidenciaVecina<'_>> {
        if !self.pasajes_de(id).is_empty() {
            return Vec::new();
        }
        let mut grupos: Vec<EvidenciaVecina<'_>> = Vec::new();
        match id {
            CombinacionId::PlanetaSigno { planeta, signo } => {
                grupos.push(EvidenciaVecina {
                    comparte: format!("planeta {planeta}"),
                    pasajes: self.pasajes_donde(|c| combinacion_usa_planeta(c, planeta)),
                });
                grupos.push(EvidenciaVecina {
                    comparte: format!("signo {signo}"),
                    pasajes: self.pasajes_donde(|c| {
                        matches!(c, CombinacionId::PlanetaSigno { signo: s, .. } if s == signo)
                    }),
                });
            }
            CombinacionId::PlanetaCasa { planeta, casa } => {
                grupos.push(EvidenciaVecina {
                    comparte: format!("planeta {planeta}"),
                    pasajes: self.pasajes_donde(|c| combinacion_usa_planeta(c, planeta)),
                });
                grupos.push(EvidenciaVecina {
                    comparte: format!("casa {casa}"),
                    pasajes: self.pasajes_donde(|c| {
                        matches!(c, CombinacionId::PlanetaCasa { casa: k, .. } if k == casa)
                    }),
                });
            }
            CombinacionId::Aspecto { a, kind, b } => {
                grupos.push(EvidenciaVecina {
                    comparte: format!("aspecto {kind}"),
                    pasajes: self.pasajes_donde(|c| {
                        matches!(c, CombinacionId::Aspecto { kind: k, .. } if k == kind)
                    }),
                });
                grupos.push(EvidenciaVecina {
                    comparte: format!("planeta {a}"),
                    pasajes: self.pasajes_donde(|c| combinacion_usa_planeta(c, a)),
                });
                grupos.push(EvidenciaVecina {
                    comparte: format!("planeta {b}"),
                    pasajes: self.pasajes_donde(|c| combinacion_usa_planeta(c, b)),
                });
            }
        }
        grupos.retain(|g| !g.pasajes.is_empty());
        grupos
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

    #[test]
    fn combinacion_id_roundtrip_string() {
        for id in [
            CombinacionId::planeta_signo("venus", "leo"),
            CombinacionId::planeta_casa("sun", 10),
            CombinacionId::aspecto("moon", "trine", "jupiter"),
        ] {
            let s = id.to_string();
            let vuelta: CombinacionId = s.parse().expect("parsea su propio Display");
            assert_eq!(vuelta, id);
        }
    }

    #[test]
    fn barra_es_alias_ascii_del_punto_medio() {
        assert_eq!(
            "mars/virgo".parse::<CombinacionId>().unwrap(),
            CombinacionId::planeta_signo("mars", "virgo"),
        );
    }

    /// Una carta mínima: Marte en Virgo en casa 6 (Social), Saturno en
    /// Aries en casa 1 (Vital), y una cuadratura que los une.
    fn carta_de_prueba() -> (Vec<Colocacion>, Vec<AspectoEnCarta>) {
        let colocaciones = vec![
            Colocacion {
                planeta: "mars".into(),
                signo: "virgo".into(),
                casa: 6,
            },
            Colocacion {
                planeta: "saturn".into(),
                signo: "aries".into(),
                casa: 1,
            },
        ];
        let aspectos = vec![AspectoEnCarta {
            a: "mars".into(),
            kind: "square".into(),
            b: "saturn".into(),
        }];
        (colocaciones, aspectos)
    }

    #[test]
    fn combinaciones_de_carta_deriva_signo_casa_y_aspectos() {
        let (colocaciones, aspectos) = carta_de_prueba();
        let combos = combinaciones_de_carta(&colocaciones, &aspectos);
        // 2 planetas × (signo + casa) + 1 aspecto.
        assert_eq!(combos.len(), 5);
        assert!(combos.contains(&CombinacionId::planeta_signo("mars", "virgo")));
        assert!(combos.contains(&CombinacionId::planeta_casa("saturn", 1)));
        assert!(combos.contains(&CombinacionId::aspecto("mars", "square", "saturn")));
    }

    #[test]
    fn rebanar_por_dominio_reparte_y_el_aspecto_puentea() {
        let (colocaciones, aspectos) = carta_de_prueba();
        let combos = combinaciones_de_carta(&colocaciones, &aspectos);
        let tajadas = rebanar_por_dominio(&colocaciones, &combos);

        // Marte en casa 6 → Social ; Saturno en casa 1 → Vital.
        let social = tajadas.get(&Dominio::Social).expect("hay tajada social");
        let vital = tajadas.get(&Dominio::Vital).expect("hay tajada vital");
        assert_eq!(social.len(), 3);
        assert_eq!(vital.len(), 3);

        // El aspecto cruza los dos planos: sale en las dos tajadas.
        let aspecto = CombinacionId::aspecto("mars", "square", "saturn");
        assert!(social.contains(&aspecto));
        assert!(vital.contains(&aspecto));
    }

    #[test]
    fn interpretar_por_dominio_agrupa_pasajes() {
        let (colocaciones, aspectos) = carta_de_prueba();
        let corpus = Corpus {
            arquetipos: Vec::new(),
            pasajes: vec![
                pasaje(CombinacionId::planeta_casa("mars", 6), "trabajo intenso"),
                pasaje(CombinacionId::planeta_casa("saturn", 1), "cuerpo severo"),
            ],
        };
        let por_dominio = corpus.interpretar_por_dominio(&colocaciones, &aspectos);
        assert_eq!(por_dominio[&Dominio::Social].len(), 1);
        assert_eq!(por_dominio[&Dominio::Vital].len(), 1);
        assert_eq!(por_dominio[&Dominio::Social][0].texto, "trabajo intenso");
    }

    #[test]
    fn ejemplo_ron_carga() {
        let corpus = Corpus::desde_ron(include_str!("../ejemplo.ron"))
            .expect("ejemplo.ron debe ser RON válido");
        assert!(!corpus.arquetipos.is_empty(), "la plantilla trae arquetipos");
        assert!(!corpus.pasajes.is_empty(), "la plantilla trae pasajes");
        // El pasaje del aspecto fija su dominio explícitamente.
        let aspecto = CombinacionId::aspecto("mars", "square", "saturn");
        let p = corpus.pasajes_de(&aspecto);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].dominio, Some(Dominio::Psiquico));
    }

    #[test]
    fn evidencia_relacionada_junta_vecinos_por_componente() {
        let corpus = Corpus {
            arquetipos: Vec::new(),
            pasajes: vec![
                pasaje(CombinacionId::planeta_signo("mars", "virgo"), "marte cirujano"),
                pasaje(CombinacionId::planeta_signo("mars", "aries"), "marte crudo"),
                pasaje(CombinacionId::planeta_signo("venus", "gemini"), "venus locuaz"),
            ],
        };
        // mars·gemini no tiene pasaje propio → evidencia vecina.
        let ev = corpus.evidencia_relacionada(&CombinacionId::planeta_signo("mars", "gemini"));
        let mars = ev.iter().find(|g| g.comparte == "planeta mars").unwrap();
        assert_eq!(mars.pasajes.len(), 2, "marte en otros signos");
        let gem = ev.iter().find(|g| g.comparte == "signo gemini").unwrap();
        assert_eq!(gem.pasajes.len(), 1, "otros planetas en géminis");
    }

    #[test]
    fn evidencia_relacionada_vacia_si_hay_pasaje_propio() {
        let corpus = Corpus {
            arquetipos: Vec::new(),
            pasajes: vec![pasaje(CombinacionId::planeta_signo("mars", "virgo"), "x")],
        };
        let ev = corpus.evidencia_relacionada(&CombinacionId::planeta_signo("mars", "virgo"));
        assert!(ev.is_empty(), "con pasaje propio no se busca evidencia vecina");
    }
}
