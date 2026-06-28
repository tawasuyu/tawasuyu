//! `pluma-cotejo` — comparar dos cuerpos *al estilo pluma*: como dos lienzos
//! paralelos del mismo material, con la diferencia de cada sección en el medio.
//!
//! El cotejo no es un `diff` de líneas: trabaja a nivel de **párrafo-átomo**,
//! igual que el resto de pluma. Alinea los párrafos de dos cuerpos por
//! **similitud léxica** (alineamiento global monótono, estilo Needleman–Wunsch),
//! clasifica cada sección resultante —idéntica, similar, divergente, agregada,
//! eliminada— y produce:
//!
//!   1. una [`CartaHebras`] `izq↔der` con `fuerza = similitud` por par
//!      (la UI ya la pinta como cinta; cuanto más fina, más divergente);
//!   2. un mapa `Uuid → divergencia ∈ [0,1]` que la UI usa para teñir cada
//!      sección de **verde** (match) a **rojo** (divergencia fuerte);
//!   3. opcionalmente, un **lienzo de diferencias** ([`columna_diferencias`]):
//!      un tercer cuerpo intercambiable cuyos átomos resumen, sección por
//!      sección, *qué* cambió — con un [`ResumidorDiferencia`] enchufable
//!      (textual determinista por defecto; un resumidor IA puede sustituirlo).
//!
//! El crate es agnóstico de UI: sólo modela. `pluma-editor-llimphi` consume
//! el mapa de divergencias para el coloreado verde→rojo del multilienzo.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use pluma_align::{Alineamiento, CartaHebras, OrigenAlineamiento};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion};
use uuid::Uuid;

/// Índice `Uuid → &NarrativeAtom`, igual contrato que el del multilienzo: el
/// cotejo no resuelve textos por su cuenta, los recibe ya indexados.
pub type IndiceAtoms<'a> = HashMap<Uuid, &'a NarrativeAtom>;

/// Cómo cambió una sección al pasar del cuerpo izquierdo al derecho.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaseCambio {
    /// Mismo texto a ambos lados (similitud ≥ `umbral_identico`).
    Identica,
    /// Texto emparentado: reformulado, corregido (similitud ≥ `umbral_similar`).
    Similar,
    /// Emparejados por posición pero con poco en común — reescritura fuerte.
    Divergente,
    /// Sólo existe a la derecha: párrafo nuevo.
    Agregada,
    /// Sólo existe a la izquierda: párrafo borrado.
    Eliminada,
}

impl ClaseCambio {
    /// `true` si la sección es un emparejamiento real entre ambos cuerpos
    /// (tiene átomo a izquierda *y* derecha).
    pub fn es_par(self) -> bool {
        matches!(self, ClaseCambio::Identica | ClaseCambio::Similar | ClaseCambio::Divergente)
    }

    /// Rótulo corto para UI/logs.
    pub fn rotulo(self) -> &'static str {
        match self {
            ClaseCambio::Identica => "idéntica",
            ClaseCambio::Similar => "similar",
            ClaseCambio::Divergente => "divergente",
            ClaseCambio::Agregada => "agregada",
            ClaseCambio::Eliminada => "eliminada",
        }
    }
}

/// Una sección del cotejo: un párrafo del izquierdo, su correspondiente del
/// derecho (si lo hay), y la fuerza de la correspondencia.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeccionCotejo {
    /// Átomo del cuerpo izquierdo, si la sección existe ahí.
    pub izq: Option<Uuid>,
    /// Átomo del cuerpo derecho, si la sección existe ahí.
    pub der: Option<Uuid>,
    /// Similitud `[0,1]` del par (1.0 = idéntico). Para agregada/eliminada
    /// es `0.0`: no hay contraparte con que comparar.
    pub similitud: f32,
    /// Clasificación derivada de `similitud` y de la presencia de contrapartes.
    pub clase: ClaseCambio,
}

impl SeccionCotejo {
    /// Divergencia `[0,1]`: 0 = idéntica, 1 = máxima. Es `1 - similitud` para
    /// los pares y `1.0` para agregadas/eliminadas (no hay nada que coincida).
    pub fn divergencia(&self) -> f32 {
        if self.clase.es_par() {
            (1.0 - self.similitud).clamp(0.0, 1.0)
        } else {
            1.0
        }
    }
}

/// Parámetros del cotejo. Los umbrales fijan la frontera entre clases; la
/// penalización de hueco regula cuándo el alineador prefiere dejar un párrafo
/// sin pareja (agregado/eliminado) en vez de emparejarlo con uno poco afín.
#[derive(Debug, Clone, Copy)]
pub struct ParamsCotejo {
    /// Similitud a partir de la cual una sección se considera idéntica.
    pub umbral_identico: f32,
    /// Similitud a partir de la cual una sección se considera similar (y por
    /// debajo, divergente).
    pub umbral_similar: f32,
    /// Penalización por dejar un párrafo sin pareja, en las mismas unidades
    /// que la similitud. Un par cuya similitud supere `2·penalizacion_hueco`
    /// se prefiere a abrir dos huecos.
    pub penalizacion_hueco: f32,
}

impl Default for ParamsCotejo {
    fn default() -> Self {
        Self {
            umbral_identico: 0.97,
            umbral_similar: 0.55,
            penalizacion_hueco: 0.45,
        }
    }
}

/// Resultado del cotejo de dos cuerpos.
#[derive(Debug, Clone)]
pub struct Cotejo {
    /// Hebras `izq↔der`: una por cada sección emparejada, con `fuerza =
    /// similitud`. Lista para el carril del multilienzo.
    pub carta: CartaHebras,
    /// Las secciones en orden de lectura (monótono respecto a ambos cuerpos).
    pub secciones: Vec<SeccionCotejo>,
    /// `Uuid → divergencia ∈ [0,1]` para *todos* los átomos de ambos cuerpos.
    /// La UI lo usa para teñir cada bloque de verde (0) a rojo (1).
    pub divergencias: HashMap<Uuid, f32>,
}

impl Cotejo {
    /// Cuántas secciones hay de cada clase. Útil para una línea de resumen.
    pub fn conteos(&self) -> Conteos {
        let mut c = Conteos::default();
        for s in &self.secciones {
            match s.clase {
                ClaseCambio::Identica => c.identicas += 1,
                ClaseCambio::Similar => c.similares += 1,
                ClaseCambio::Divergente => c.divergentes += 1,
                ClaseCambio::Agregada => c.agregadas += 1,
                ClaseCambio::Eliminada => c.eliminadas += 1,
            }
        }
        c
    }
}

/// Recuento de secciones por clase.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Conteos {
    pub identicas: usize,
    pub similares: usize,
    pub divergentes: usize,
    pub agregadas: usize,
    pub eliminadas: usize,
}

// =============================================================================
//  Similitud léxica
// =============================================================================

/// Similitud léxica `[0,1]` entre dos textos: coeficiente de Sørensen–Dice
/// sobre el **multiconjunto de palabras** normalizadas (minúsculas, partido
/// por no-alfanuméricos Unicode). 1.0 = mismas palabras con las mismas
/// repeticiones; 0.0 = ninguna palabra en común. Es robusto a reordenamientos
/// y a ediciones puntuales, e ignora puntuación y mayúsculas.
///
/// Dos textos vacíos se consideran idénticos (1.0); uno vacío frente a otro no
/// vacío, disjuntos (0.0).
pub fn similitud(a: &str, b: &str) -> f32 {
    let ta = tokenizar(a);
    let tb = tokenizar(b);
    if ta.is_empty() && tb.is_empty() {
        return 1.0;
    }
    if ta.is_empty() || tb.is_empty() {
        return 0.0;
    }
    let mut bolsa: HashMap<&str, i32> = HashMap::new();
    for t in &ta {
        *bolsa.entry(t.as_str()).or_insert(0) += 1;
    }
    let mut interseccion = 0usize;
    for t in &tb {
        let e = bolsa.entry(t.as_str()).or_insert(0);
        if *e > 0 {
            *e -= 1;
            interseccion += 1;
        }
    }
    (2.0 * interseccion as f32) / (ta.len() + tb.len()) as f32
}

/// Parte un texto en palabras normalizadas: minúsculas, separadas por
/// cualquier carácter no alfanumérico (Unicode-aware). Sin asignar de más:
/// devuelve `String`s ya en minúscula para comparación directa.
fn tokenizar(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

// =============================================================================
//  Cotejo: alineamiento global por similitud
// =============================================================================

/// Cotejá dos cuerpos. Alinea sus párrafos maximizando la similitud total con
/// huecos penalizados (Needleman–Wunsch sobre la matriz de similitudes), luego
/// clasifica cada sección y arma la carta de hebras y el mapa de divergencias.
///
/// `ahora` es el timestamp que llevan las hebras emitidas (origen `Manual`,
/// autor `"cotejo"` — la comparación es una lectura, no una derivación).
pub fn cotejar(
    izq: &Cuerpo,
    der: &Cuerpo,
    atoms: &IndiceAtoms<'_>,
    params: &ParamsCotejo,
    ahora: u64,
) -> Cotejo {
    let texto = |id: &Uuid| -> &str {
        atoms.get(id).map(|a| a.content.as_str()).unwrap_or("")
    };

    let a: Vec<Uuid> = izq.orden.clone();
    let b: Vec<Uuid> = der.orden.clone();
    let m = a.len();
    let n = b.len();

    // Matriz de similitudes a[i] vs b[j].
    let mut sim = vec![0.0f32; m * n];
    for i in 0..m {
        for j in 0..n {
            sim[i * n + j] = similitud(texto(&a[i]), texto(&b[j]));
        }
    }

    let gap = params.penalizacion_hueco;

    // DP con matriz de decisiones (0 = diagonal/par, 1 = hueco izq/eliminada,
    // 2 = hueco der/agregada). Evita comparar floats en el traceback.
    let mut dp = vec![0.0f32; (m + 1) * (n + 1)];
    let mut dir = vec![0u8; (m + 1) * (n + 1)];
    let idx = |i: usize, j: usize| i * (n + 1) + j;
    for i in 1..=m {
        dp[idx(i, 0)] = dp[idx(i - 1, 0)] - gap;
        dir[idx(i, 0)] = 1;
    }
    for j in 1..=n {
        dp[idx(0, j)] = dp[idx(0, j - 1)] - gap;
        dir[idx(0, j)] = 2;
    }
    for i in 1..=m {
        for j in 1..=n {
            let diag = dp[idx(i - 1, j - 1)] + sim[(i - 1) * n + (j - 1)];
            let arriba = dp[idx(i - 1, j)] - gap;
            let izqui = dp[idx(i, j - 1)] - gap;
            let (mejor, d) = if diag >= arriba && diag >= izqui {
                (diag, 0u8)
            } else if arriba >= izqui {
                (arriba, 1u8)
            } else {
                (izqui, 2u8)
            };
            dp[idx(i, j)] = mejor;
            dir[idx(i, j)] = d;
        }
    }

    // Traceback → secciones (en orden inverso, luego se revierte).
    let mut secciones: Vec<SeccionCotejo> = Vec::new();
    let (mut i, mut j) = (m, n);
    while i > 0 || j > 0 {
        match dir[idx(i, j)] {
            0 => {
                let s = sim[(i - 1) * n + (j - 1)];
                secciones.push(SeccionCotejo {
                    izq: Some(a[i - 1]),
                    der: Some(b[j - 1]),
                    similitud: s,
                    clase: clasificar(s, params),
                });
                i -= 1;
                j -= 1;
            }
            1 => {
                secciones.push(SeccionCotejo {
                    izq: Some(a[i - 1]),
                    der: None,
                    similitud: 0.0,
                    clase: ClaseCambio::Eliminada,
                });
                i -= 1;
            }
            _ => {
                secciones.push(SeccionCotejo {
                    izq: None,
                    der: Some(b[j - 1]),
                    similitud: 0.0,
                    clase: ClaseCambio::Agregada,
                });
                j -= 1;
            }
        }
    }
    secciones.reverse();

    // Carta de hebras + mapa de divergencias.
    let mut carta = CartaHebras::nueva().con_par(izq.id, der.id);
    let mut divergencias: HashMap<Uuid, f32> = HashMap::new();
    for s in &secciones {
        let d = s.divergencia();
        if let Some(ia) = s.izq {
            divergencias.insert(ia, d);
        }
        if let Some(ib) = s.der {
            divergencias.insert(ib, d);
        }
        if let (Some(ia), Some(ib)) = (s.izq, s.der) {
            carta.agregar(Alineamiento::nuevo(
                ia,
                ib,
                s.similitud,
                OrigenAlineamiento::Manual {
                    autor: "cotejo".into(),
                    timestamp: ahora,
                },
            ));
        }
    }

    Cotejo {
        carta,
        secciones,
        divergencias,
    }
}

/// Clasifica una sección emparejada por su similitud y los umbrales.
fn clasificar(s: f32, p: &ParamsCotejo) -> ClaseCambio {
    if s >= p.umbral_identico {
        ClaseCambio::Identica
    } else if s >= p.umbral_similar {
        ClaseCambio::Similar
    } else {
        ClaseCambio::Divergente
    }
}

// =============================================================================
//  Lienzo de diferencias (tercer cuerpo intercambiable)
// =============================================================================

/// Quien redacta el texto de cada sección del lienzo de diferencias. El
/// default [`ResumidorTextual`] es determinista y sin red; un resumidor IA
/// (que llame a `pluma-llm`) puede implementar este trait para producir el
/// "resumen semántico o inteligente" de cada diferencia.
pub trait ResumidorDiferencia {
    /// Redacta la línea de la sección. `izq`/`der` son los textos de las
    /// contrapartes (los que existan).
    fn resumir(&self, sec: &SeccionCotejo, izq: Option<&str>, der: Option<&str>) -> String;
}

/// Resumidor determinista: un glifo de clase + una nota corta. Sin red, apto
/// para tests y para el render por defecto.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResumidorTextual;

impl ResumidorDiferencia for ResumidorTextual {
    fn resumir(&self, sec: &SeccionCotejo, izq: Option<&str>, der: Option<&str>) -> String {
        let pct = (sec.similitud * 100.0).round() as i32;
        match sec.clase {
            ClaseCambio::Identica => "≡ sin cambios".to_string(),
            ClaseCambio::Similar => format!("≈ reformulado · {pct}% en común"),
            ClaseCambio::Divergente => format!("✗ reescrito · {pct}% en común"),
            ClaseCambio::Agregada => {
                format!("＋ agregado: {}", recorte(der.unwrap_or("")))
            }
            ClaseCambio::Eliminada => {
                format!("－ eliminado: {}", recorte(izq.unwrap_or("")))
            }
        }
    }
}

/// Recorta a un preview corto de una línea (sin romper UTF-8).
fn recorte(s: &str) -> String {
    const LIM: usize = 80;
    let mut t = s.replace('\n', " ");
    if t.chars().count() > LIM {
        t = t.chars().take(LIM).collect::<String>();
        t.push('…');
    }
    t
}

/// El lienzo de diferencias y sus cartas hacia ambos lados, listo para montar
/// como columna del medio en el multilienzo (`[izq, diferencias, der]`).
#[derive(Debug, Clone)]
pub struct ColumnaDiferencias {
    /// El cuerpo "diferencias": un átomo por sección, en orden.
    pub cuerpo: Cuerpo,
    /// Los átomos del cuerpo de diferencias (el caller debe mantenerlos vivos
    /// e indexarlos junto a los demás).
    pub atoms: Vec<NarrativeAtom>,
    /// Hebras `izq ↔ diferencias` (una por sección con contraparte izquierda).
    pub carta_izq: CartaHebras,
    /// Hebras `diferencias ↔ der` (una por sección con contraparte derecha).
    pub carta_der: CartaHebras,
    /// Divergencia de cada átomo de diferencias (= divergencia de su sección).
    pub divergencias: HashMap<Uuid, f32>,
}

/// Construí el lienzo de diferencias a partir de un [`Cotejo`]. Cada sección
/// produce un átomo cuyo texto lo redacta `resumidor`. Las cartas conectan ese
/// átomo con sus contrapartes izquierda/derecha (fuerza = similitud de la
/// sección, o 1.0 para agregada/eliminada: la *existencia* del vínculo es
/// segura aunque el contenido difiera).
pub fn columna_diferencias<R: ResumidorDiferencia>(
    cotejo: &Cotejo,
    izq: &Cuerpo,
    der: &Cuerpo,
    atoms: &IndiceAtoms<'_>,
    resumidor: &R,
    ahora: u64,
) -> ColumnaDiferencias {
    let texto = |id: &Uuid| -> Option<&str> { atoms.get(id).map(|a| a.content.as_str()) };

    let mut cuerpo = Cuerpo::nuevo(
        "diferencias",
        "diferencias",
        Intencion::Custom {
            kind: "cotejo".into(),
        },
        ahora,
    );
    let mut nuevos: Vec<NarrativeAtom> = Vec::with_capacity(cotejo.secciones.len());
    let mut carta_izq = CartaHebras::nueva().con_par(izq.id, cuerpo.id);
    let mut carta_der = CartaHebras::nueva().con_par(cuerpo.id, der.id);
    let mut divergencias: HashMap<Uuid, f32> = HashMap::new();

    let origen = |ts: u64| OrigenAlineamiento::Manual {
        autor: "cotejo".into(),
        timestamp: ts,
    };

    for sec in &cotejo.secciones {
        let linea = resumidor.resumir(
            sec,
            sec.izq.as_ref().and_then(|id| texto(id)),
            sec.der.as_ref().and_then(|id| texto(id)),
        );
        let atom = NarrativeAtom::new(linea, "diferencias");
        let aid = atom.id;
        cuerpo.agregar(aid, ahora);
        divergencias.insert(aid, sec.divergencia());

        // Vínculo con la izquierda: fuerza = similitud de la sección si es un
        // par; para eliminada el vínculo existe (1.0) — es el lado que aporta.
        if let Some(ia) = sec.izq {
            let f = if sec.clase.es_par() { sec.similitud } else { 1.0 };
            carta_izq.agregar(Alineamiento::nuevo(ia, aid, f, origen(ahora)));
        }
        if let Some(ib) = sec.der {
            let f = if sec.clase.es_par() { sec.similitud } else { 1.0 };
            carta_der.agregar(Alineamiento::nuevo(aid, ib, f, origen(ahora)));
        }

        nuevos.push(atom);
    }

    ColumnaDiferencias {
        cuerpo,
        atoms: nuevos,
        carta_izq,
        carta_der,
        divergencias,
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn cuerpo_con(textos: &[&str], branch: &str) -> (Cuerpo, Vec<NarrativeAtom>) {
        let mut c = Cuerpo::nuevo(branch, branch, Intencion::Original, 0);
        let atoms: Vec<NarrativeAtom> =
            textos.iter().map(|t| NarrativeAtom::new(*t, branch)).collect();
        for a in &atoms {
            c.agregar(a.id, 0);
        }
        (c, atoms)
    }

    fn indice<'a>(grupos: &[&'a [NarrativeAtom]]) -> IndiceAtoms<'a> {
        let mut idx = IndiceAtoms::new();
        for g in grupos {
            for a in g.iter() {
                idx.insert(a.id, a);
            }
        }
        idx
    }

    #[test]
    fn similitud_idesnticos_y_disjuntos() {
        assert!((similitud("hola mundo", "hola mundo") - 1.0).abs() < 1e-6);
        assert_eq!(similitud("perro gato", "luna estrella"), 0.0);
        assert_eq!(similitud("", ""), 1.0);
        assert_eq!(similitud("algo", ""), 0.0);
    }

    #[test]
    fn similitud_ignora_orden_y_puntuacion() {
        // Mismas palabras, otro orden y puntuación → 1.0.
        let s = similitud("El gato, come pescado.", "pescado come gato el");
        assert!((s - 1.0).abs() < 1e-6, "s={s}");
    }

    #[test]
    fn similitud_edicion_puntual_es_parcial() {
        // 3 de 4 palabras en común → Dice = 2*3/(4+4) = 0.75.
        let s = similitud("el gato come pescado", "el gato comió pescado");
        assert!((s - 0.75).abs() < 1e-6, "s={s}");
    }

    #[test]
    fn cotejar_empareja_identicos_y_marca_divergencia() {
        let (izq, ai) = cuerpo_con(&["uno dos tres", "cuatro cinco seis"], "a");
        let (der, ad) = cuerpo_con(&["uno dos tres", "cuatro cinco SEIS distinto"], "b");
        let idx = indice(&[&ai, &ad]);
        let cot = cotejar(&izq, &der, &idx, &ParamsCotejo::default(), 1);

        assert_eq!(cot.secciones.len(), 2);
        assert_eq!(cot.secciones[0].clase, ClaseCambio::Identica);
        assert!((cot.secciones[0].divergencia() - 0.0).abs() < 1e-6);
        // La segunda es par pero con menos en común → más divergente que la
        // primera, y emparejada (no agregada/eliminada).
        assert!(cot.secciones[1].clase.es_par());
        assert!(cot.secciones[1].divergencia() > cot.secciones[0].divergencia());

        // Dos hebras (ambas secciones son pares).
        assert_eq!(cot.carta.hebras.len(), 2);
        // Divergencias cubren los 4 átomos.
        assert_eq!(cot.divergencias.len(), 4);
        assert!((cot.divergencias[&ai[0].id] - 0.0).abs() < 1e-6);
    }

    #[test]
    fn cotejar_detecta_agregado_y_eliminado() {
        // der inserta un párrafo en el medio; izq tiene uno que der no.
        let (izq, ai) = cuerpo_con(&["alfa beta", "gamma delta", "solo izquierda"], "a");
        let (der, ad) = cuerpo_con(&["alfa beta", "nuevo intermedio", "gamma delta"], "b");
        let idx = indice(&[&ai, &ad]);
        let cot = cotejar(&izq, &der, &idx, &ParamsCotejo::default(), 1);

        let c = cot.conteos();
        assert_eq!(c.identicas, 2, "alfa/beta y gamma/delta deben anclar");
        assert_eq!(c.agregadas, 1, "‘nuevo intermedio’ es agregado");
        assert_eq!(c.eliminadas, 1, "‘solo izquierda’ es eliminado");

        // Las hebras sólo cubren las 2 secciones emparejadas.
        assert_eq!(cot.carta.hebras.len(), 2);
        // El átomo agregado y el eliminado tienen divergencia 1.0.
        let agr = cot.secciones.iter().find(|s| s.clase == ClaseCambio::Agregada).unwrap();
        assert!((cot.divergencias[&agr.der.unwrap()] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn columna_diferencias_tiene_un_atomo_por_seccion() {
        let (izq, ai) = cuerpo_con(&["uno dos", "tres cuatro"], "a");
        let (der, ad) = cuerpo_con(&["uno dos", "tres cuatro cinco", "seis siete"], "b");
        let idx = indice(&[&ai, &ad]);
        let cot = cotejar(&izq, &der, &idx, &ParamsCotejo::default(), 1);
        let col = columna_diferencias(&cot, &izq, &der, &idx, &ResumidorTextual, 2);

        assert_eq!(col.atoms.len(), cot.secciones.len());
        assert_eq!(col.cuerpo.orden.len(), cot.secciones.len());
        // Cada átomo de diferencias tiene su divergencia registrada.
        for a in &col.atoms {
            assert!(col.divergencias.contains_key(&a.id));
        }
        // La sección idéntica produce la línea "sin cambios".
        let idx_dif: IndiceAtoms = col.atoms.iter().map(|a| (a.id, a)).collect();
        let primera = col.cuerpo.orden[0];
        assert!(idx_dif[&primera].content.contains("sin cambios"));
    }

    #[test]
    fn cotejo_de_cuerpos_vacios_no_panica() {
        let (izq, ai) = cuerpo_con(&[], "a");
        let (der, ad) = cuerpo_con(&[], "b");
        let idx = indice(&[&ai, &ad]);
        let cot = cotejar(&izq, &der, &idx, &ParamsCotejo::default(), 1);
        assert!(cot.secciones.is_empty());
        assert!(cot.carta.hebras.is_empty());
    }
}
