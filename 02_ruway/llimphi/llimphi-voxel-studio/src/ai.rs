//! Conexión IA **"poor"** del studio. Convierte prosa en artefactos del modelo:
//! un [`Bioma`] (relieve + materiales), una [`SceneSpec`] o un [`CharSpec`]. Dos
//! caminos en cascada donde aplica: LLM real (`pluma-llm`, autodetecta backend) y,
//! si no hay modelo o falla el parseo, una **heurística local** offline — así el
//! botón siempre produce algo.
//!
//! Tras el rediseño por niveles, los materiales son **autorables** (ids en el
//! `Project`), así que la generación de un bioma referencia los materiales semilla
//! vía [`MatRefs`] en vez de un `enum` cerrado.

use llimphi_voxel::{Bioma, CharSpec, Clip, Forma, ObjetoUso, Project, SceneSpec};
use pluma_llm::pluma_llm_core::ChatRequest;
use serde::Deserialize;

/// Ids de los materiales **semilla** del proyecto que un bioma puede referenciar.
/// Se resuelven una vez (con [`from_project`](Self::from_project)) y se capturan en
/// el worker de IA — así la generación no necesita el `Project` entero.
#[derive(Debug, Clone, Copy)]
pub struct MatRefs {
    pub sand: u64,
    pub grass: u64,
    pub rock: u64,
    pub snow: u64,
    pub cactus: u64,
}

impl MatRefs {
    /// Resuelve los ids semilla del proyecto (0 si falta alguno — no debería con el
    /// proyecto de arranque).
    pub fn from_project(p: &Project) -> Self {
        use llimphi_voxel::Material::*;
        let id = |m| p.material_id_for(m).unwrap_or(0);
        Self {
            sand: id(Sand),
            grass: id(Grass),
            rock: id(Rock),
            snow: id(Snow),
            cactus: id(Cactus),
        }
    }
}

/// Pregunta al LLM real (de `pluma-llm`, `from_env`) y devuelve el texto. `None` si
/// el backend es **Mock** (sin credenciales) o si la red falla. Motor compartido de
/// la asistencia; cada caller parsea la salida a su artefacto.
fn ask_llm(system: &str, prompt: &str, max_tokens: u32) -> Option<String> {
    let client = pluma_llm::from_env().ok()?;
    if client.model_id().to_lowercase().contains("mock") {
        return None;
    }
    let req = ChatRequest::una_vuelta(prompt, max_tokens)
        .con_sistema(system)
        .con_temperatura(0.5);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    rt.block_on(client.complete(&req)).ok().map(|r| r.content)
}

/// Recorta un texto al literal RON `( … )` que contenga (tolera ``` y ruido).
fn ron_slice(text: &str) -> Option<&str> {
    let start = text.find('(')?;
    let end = text.rfind(')')?;
    (end > start).then(|| &text[start..=end])
}

// =============================================================================
//  Biomas
// =============================================================================

/// **Bioma desde prosa** (heurística local por palabras clave). Arranca de un preset
/// (desierto o pradera) referenciando los materiales semilla `m` y lo ajusta según
/// términos del prompt. Offline e instantánea — el alma "poor". El id queda en 0; lo
/// asigna el `Project` al agregarlo.
pub fn generate_bioma(prompt: &str, m: &MatRefs) -> Bioma {
    let p = prompt.to_lowercase();
    let has = |k: &str| p.contains(k);

    let desierto = has("desierto") || has("arena") || has("seco") || has("duna");
    let mut b = if desierto {
        Bioma {
            id: 0,
            name: name_from(prompt, "bioma"),
            base: 0.30,
            dune: 0.05,
            relief: 0.45,
            mountains: 0.12,
            water_level: 0.26,
            rivers: 0.18,
            peak_at: 1.0,
            ground: m.sand,
            cliff: m.rock,
            peak: None,
            objetos: vec![ObjetoUso { material: m.cactus, densidad: 0.010, forma: Forma::Columnar }],
            seres: vec![],
        }
    } else {
        Bioma {
            id: 0,
            name: name_from(prompt, "bioma"),
            base: 0.22,
            dune: 0.10,
            relief: 0.7,
            mountains: 0.5,
            water_level: 0.30,
            rivers: 0.25,
            peak_at: 0.80,
            ground: m.grass,
            cliff: m.rock,
            peak: Some(m.snow),
            objetos: vec![],
            seres: vec![],
        }
    };

    if has("nieve") || has("monta") || has("montañ") || has("cumbre") || has("alpin") || has("pico") {
        b.relief = b.relief.max(0.8);
        b.mountains = b.mountains.max(0.65);
        b.peak = Some(m.snow);
        b.peak_at = 0.62;
    }
    if has("llano") || has("plano") || has("pradera") || has("planicie") || has("estepa") {
        b.relief = b.relief.min(0.35);
        b.mountains = b.mountains.min(0.2);
    }
    if has("agua") || has("lago") || has("mar") || has("océano") || has("oceano") || has("isla") || has("inund") {
        b.water_level = b.water_level.max(0.5);
    }
    if has("río") || has("rio") || has("cauce") || has("arroyo") {
        b.rivers = b.rivers.max(0.6);
    }
    if has("bosque") || has("selva") || has("verde") || has("pasto") {
        b.ground = m.grass;
    }
    if has("cactus") || desierto {
        if !b.objetos.iter().any(|o| o.material == m.cactus) {
            b.objetos.push(ObjetoUso { material: m.cactus, densidad: 0.012, forma: Forma::Columnar });
        }
    }
    b
}

// =============================================================================
//  Escenas
// =============================================================================

/// "Brief" mínimo que el LLM emite para una escena: cuántos actores y qué gesto.
#[derive(Deserialize)]
struct SceneBrief {
    actors: usize,
    gesture: Clip,
}

const SCENE_SYSTEM: &str = "\
Dirigís una escena voxel corta. Respondé SÓLO con un literal RON de la forma
(actors: N, gesture: G), sin markdown ni texto extra. N = cantidad de personajes
(1..5). G = el gesto final: uno de Idle, Walk, Run, Wave, Point, Cheer.
Ejemplo: (actors: 3, gesture: Cheer)";

/// **Escena desde prosa** en el mundo `mundo` (id): LLM real (brief) → escena patrón;
/// si no hay LLM o falla, heurística local. Siempre devuelve algo.
pub fn generate_scene(prompt: &str, mundo: u64, dim: [u32; 3]) -> SceneSpec {
    llm_scene(prompt, mundo, dim).unwrap_or_else(|| local_scene(prompt, mundo, dim))
}

fn llm_scene(prompt: &str, mundo: u64, dim: [u32; 3]) -> Option<SceneSpec> {
    let brief: SceneBrief = ron::from_str(ron_slice(&ask_llm(SCENE_SYSTEM, prompt, 80)?)?).ok()?;
    Some(SceneSpec::walk_and_emote(
        scene_name(prompt),
        mundo,
        brief.actors.clamp(1, 5),
        brief.gesture,
        dim,
    ))
}

fn local_scene(prompt: &str, mundo: u64, dim: [u32; 3]) -> SceneSpec {
    let p = prompt.to_lowercase();
    let n = parse_count(&p);
    let gesture = if p.contains("salud") {
        Clip::Wave
    } else if p.contains("festej") || p.contains("celebr") || p.contains("baila") || p.contains("fiesta") {
        Clip::Cheer
    } else if p.contains("señal") || p.contains("apunt") || p.contains("muestr") {
        Clip::Point
    } else {
        Clip::Wave
    };
    SceneSpec::walk_and_emote(scene_name(prompt), mundo, n, gesture, dim)
}

/// Cuántos actores pide el texto (palabra o dígito); 3 por defecto.
fn parse_count(p: &str) -> usize {
    for (w, n) in [
        ("cinco", 5usize), ("cuatro", 4), ("tres", 3), ("dos", 2), ("una", 1), ("uno", 1),
        (" 5", 5), (" 4", 4), (" 3", 3), (" 2", 2), (" 1", 1),
    ] {
        if p.contains(w) {
            return n;
        }
    }
    3
}

// =============================================================================
//  Personajes (seres)
// =============================================================================

const CHAR_SYSTEM: &str = "\
Generás un personaje voxel. Respondé SÓLO con un literal RON de CharSpec, sin
markdown ni texto extra. Campos: name (texto), age (Baby|Child|Teen|Adult|Elder),
skin/shirt/pants (cada uno [r, g, b] en 0..1).
Ejemplo: (name: \"rojo\", age: Adult, skin: [0.9, 0.72, 0.58], shirt: [0.82, 0.28, 0.26], pants: [0.2, 0.2, 0.28])";

/// **Personaje desde prosa**: LLM real (`CharSpec` en RON) y, si no hay o falla, la
/// heurística local. Siempre devuelve algo (id 0; lo asigna el `Project`).
pub fn generate_character(prompt: &str) -> CharSpec {
    llm_character(prompt).unwrap_or_else(|| local_character(prompt))
}

fn llm_character(prompt: &str) -> Option<CharSpec> {
    ron::from_str::<CharSpec>(ron_slice(&ask_llm(CHAR_SYSTEM, prompt, 200)?)?).ok()
}

fn local_character(prompt: &str) -> CharSpec {
    use llimphi_voxel::Age;
    let p = prompt.to_lowercase();
    let age = if p.contains("bebé") || p.contains("bebe") || p.contains("recién") || p.contains("recien") {
        Age::Baby
    } else if p.contains("niñ") || p.contains("nin") || p.contains("chic") {
        Age::Child
    } else if p.contains("joven") || p.contains("adolesc") || p.contains("teen") {
        Age::Teen
    } else if p.contains("ancian") || p.contains("viej") || p.contains("abuel") {
        Age::Elder
    } else {
        Age::Adult
    };
    let mut c = CharSpec::new(scene_name(prompt), age);
    if let Some(rgb) = parse_color(&p) {
        c.shirt = rgb;
    }
    c
}

/// Color (`[r,g,b]` en `[0,1]`) nombrado en el texto, si lo hay.
fn parse_color(p: &str) -> Option<[f32; 3]> {
    let table: [(&str, [f32; 3]); 12] = [
        ("roj", [0.82, 0.28, 0.26]),
        ("celest", [0.50, 0.75, 0.92]),
        ("azul", [0.22, 0.55, 0.78]),
        ("verd", [0.30, 0.70, 0.40]),
        ("amarill", [0.92, 0.80, 0.30]),
        ("naranj", [0.90, 0.50, 0.22]),
        ("violet", [0.62, 0.40, 0.78]),
        ("morad", [0.62, 0.40, 0.78]),
        ("rosa", [0.90, 0.55, 0.70]),
        ("blanc", [0.92, 0.92, 0.92]),
        ("negr", [0.14, 0.14, 0.16]),
        ("marr", [0.45, 0.32, 0.22]),
    ];
    table.iter().find(|(k, _)| p.contains(k)).map(|(_, c)| *c)
}

/// Nombre de escena = primeras ~4 palabras de la descripción.
fn scene_name(prompt: &str) -> String {
    name_from(prompt, "escena IA")
}

/// Nombre a partir de las primeras ~4 palabras del prompt, con un default.
fn name_from(prompt: &str, default: &str) -> String {
    let s: String = prompt.split_whitespace().take(4).collect::<Vec<_>>().join(" ");
    if s.is_empty() {
        default.into()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs() -> MatRefs {
        MatRefs::from_project(&Project::starter())
    }

    #[test]
    fn heuristica_detecta_desierto_con_cactus() {
        let m = refs();
        let b = generate_bioma("un desierto enorme con cactus", &m);
        assert_eq!(b.ground, m.sand);
        assert!(b.objetos.iter().any(|o| o.material == m.cactus));
    }

    #[test]
    fn heuristica_detecta_montañas_nevadas() {
        let m = refs();
        let b = generate_bioma("cordillera con picos de nieve", &m);
        assert_eq!(b.peak, Some(m.snow));
        assert!(b.mountains >= 0.65);
    }

    #[test]
    fn bioma_referencia_materiales_del_proyecto() {
        let p = Project::starter();
        let m = MatRefs::from_project(&p);
        let b = generate_bioma("pradera verde con ríos", &m);
        // Los ids referenciados existen como materiales del proyecto.
        assert!(p.material(b.ground).is_some());
        assert!(p.material(b.cliff).is_some());
    }

    #[test]
    fn personaje_lee_edad_y_color() {
        use llimphi_voxel::Age;
        let c = local_character("un niño de remera roja");
        assert_eq!(c.age, Age::Child);
        assert_eq!(c.shirt, [0.82, 0.28, 0.26]);
    }

    #[test]
    fn escena_lee_cantidad_y_gesto() {
        let s = local_scene("dos personajes que festejan", 7, [128, 56, 128]);
        assert_eq!(s.actors.len(), 2);
        assert_eq!(s.mundo, 7);
        let tiene_cheer = s.actors[0].keys.iter().any(|k| k.clip == Some(Clip::Cheer));
        assert!(tiene_cheer, "festejar → Cheer");
    }
}
