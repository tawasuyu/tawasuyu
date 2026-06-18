//! Conexión IA **"poor"** del studio: convierte una descripción en prosa en una
//! [`WorldRecipe`]. Dos caminos, en cascada:
//!
//! 1. **LLM** vía `pluma-llm` (`from_env`, autodetecta backend por env). Se le pide
//!    que emita la receta como literal RON; se parsea contra el struct real.
//! 2. **Heurística local** por palabras clave — instantánea y offline. Es el
//!    fallback cuando no hay modelo real (Mock) o la salida del LLM no parsea, así
//!    el botón **siempre** produce un mundo (de ahí lo "poor": sirve sin nube).

use llimphi_voxel::{Flora, Material, WorldRecipe};
use pluma_llm::pluma_llm_core::ChatRequest;

/// Instrucción del sistema: enseña el formato RON exacto y el significado de cada
/// parámetro, y exige responder **sólo** con el literal.
const SYSTEM: &str = "\
Sos un generador de mundos voxel. Dada una descripción en lenguaje natural, \
respondé EXCLUSIVAMENTE con un literal RON de la estructura WorldRecipe, sin \
markdown, sin ``` y sin texto extra.

Campos (todos obligatorios):
- seed: u32 (semilla; elegí uno cualquiera)
- base: f32 0..0.9 (nivel del suelo, fracción del alto)
- dune: f32 0..0.4 (amplitud de ondulaciones suaves)
- relief: f32 0..1 (alto de las montañas)
- mountains: f32 0..1 (densidad de montañas; 0 = casi llano)
- water_level: f32 0..0.9 (nivel del agua)
- rivers: f32 0..1 (densidad de ríos)
- ground: material del suelo
- cliff: material de acantilado/altura
- peak: material de cumbre (Air = sin cumbre)
- peak_at: f32 0..1 (altura desde la que aparece la cumbre)
- flora: tipo de planta
- flora_density: f32 0..0.05

Materiales válidos: Air, Sand, Grass, Rock, Snow, Water, Cactus.
Flora válida: None, Cactus.

Ejemplo (desierto llano con cactus):
(seed: 7, base: 0.3, dune: 0.05, relief: 0.45, mountains: 0.12, water_level: 0.26, rivers: 0.18, ground: Sand, cliff: Rock, peak: Air, peak_at: 1.0, flora: Cactus, flora_density: 0.01)

Ejemplo (cordillera nevada):
(seed: 42, base: 0.25, dune: 0.08, relief: 0.85, mountains: 0.7, water_level: 0.3, rivers: 0.3, ground: Grass, cliff: Rock, peak: Snow, peak_at: 0.65, flora: None, flora_density: 0.0)";

/// El RON del ejemplo del prompt — también sirve de aserción de que enseñamos un
/// literal parseable.
const SAMPLE: &str = "(seed: 7, base: 0.3, dune: 0.05, relief: 0.45, mountains: 0.12, water_level: 0.26, rivers: 0.18, ground: Sand, cliff: Rock, peak: Air, peak_at: 1.0, flora: Cactus, flora_density: 0.01)";

/// Genera una receta desde una descripción. **Siempre** devuelve algo: intenta el
/// LLM y, si no hay modelo real o falla el parseo, cae a la heurística local.
/// Bloqueante (red): llamar desde un worker (`Handle::spawn`), no en el hilo UI.
pub fn generate(prompt: &str) -> WorldRecipe {
    via_llm(prompt).unwrap_or_else(|| local_recipe(prompt))
}

/// Intenta el LLM real. `None` si el backend es Mock (sin credenciales), si la red
/// falla, o si la salida no parsea a [`WorldRecipe`].
fn via_llm(prompt: &str) -> Option<WorldRecipe> {
    let client = pluma_llm::from_env().ok()?;
    // Sin credenciales `from_env` cae a Mock: su salida no es una receta, así que
    // ni gastamos la vuelta — directo a la heurística.
    if client.model_id().to_lowercase().contains("mock") {
        return None;
    }
    let req = ChatRequest::una_vuelta(prompt, 400)
        .con_sistema(SYSTEM)
        .con_temperatura(0.5);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    let resp = rt.block_on(client.complete(&req)).ok()?;
    parse_recipe(&resp.content)
}

/// Parsea una [`WorldRecipe`] de un texto que puede traer ``` fences o ruido
/// alrededor: recorta al literal RON `( … )` y deserializa.
pub fn parse_recipe(text: &str) -> Option<WorldRecipe> {
    let start = text.find('(')?;
    let end = text.rfind(')')?;
    if end <= start {
        return None;
    }
    ron::from_str::<WorldRecipe>(&text[start..=end]).ok()
}

/// Semilla determinista por la descripción (mismo texto → mismo mundo).
fn seed_of(prompt: &str) -> u32 {
    prompt
        .bytes()
        .fold(2166136261u32, |a, b| (a ^ b as u32).wrapping_mul(16777619))
        % 100_000
}

/// **Heurística local** por palabras clave: arranca de un preset (desierto o
/// pradera) y lo ajusta según términos del prompt (nieve, agua, ríos, llano…).
/// Offline e instantánea — el alma "poor" de la asistencia.
pub fn local_recipe(prompt: &str) -> WorldRecipe {
    let p = prompt.to_lowercase();
    let has = |k: &str| p.contains(k);
    let seed = seed_of(prompt);

    let mut r = if has("desierto") || has("arena") || has("seco") || has("duna") {
        WorldRecipe::desert(seed)
    } else {
        WorldRecipe::grassland(seed)
    };

    if has("nieve") || has("monta") || has("montañ") || has("cumbre") || has("alpin") || has("pico")
    {
        r.relief = r.relief.max(0.8);
        r.mountains = r.mountains.max(0.65);
        r.peak = Material::Snow;
        r.peak_at = 0.62;
    }
    if has("llano") || has("plano") || has("pradera") || has("planicie") || has("estepa") {
        r.relief = r.relief.min(0.35);
        r.mountains = r.mountains.min(0.2);
    }
    if has("agua") || has("lago") || has("mar") || has("océano") || has("oceano") || has("isla")
        || has("inund")
    {
        r.water_level = r.water_level.max(0.5);
    }
    if has("río") || has("rio") || has("cauce") || has("arroyo") {
        r.rivers = r.rivers.max(0.6);
    }
    if has("bosque") || has("selva") || has("verde") || has("pasto") {
        r.ground = Material::Grass;
    }
    if has("cactus") || has("desierto") {
        r.flora = Flora::Cactus;
        r.flora_density = r.flora_density.max(0.012);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_ejemplo_del_prompt_es_ron_valido() {
        assert!(parse_recipe(SAMPLE).is_some(), "el ejemplo que enseñamos debe parsear");
    }

    #[test]
    fn parse_tolera_fences_y_ruido() {
        let txt = "Claro, acá va:\n```ron\n(seed: 1, base: 0.2, dune: 0.1, relief: 0.7, \
mountains: 0.5, water_level: 0.3, rivers: 0.25, ground: Grass, cliff: Rock, peak: Snow, \
peak_at: 0.8, flora: None, flora_density: 0.0)\n```\nlisto.";
        assert!(parse_recipe(txt).is_some());
    }

    #[test]
    fn heuristica_detecta_desierto_con_cactus() {
        let r = local_recipe("un desierto enorme con cactus");
        assert_eq!(r.ground, Material::Sand);
        assert_eq!(r.flora, Flora::Cactus);
    }

    #[test]
    fn heuristica_detecta_montañas_nevadas() {
        let r = local_recipe("cordillera con picos de nieve");
        assert_eq!(r.peak, Material::Snow);
        assert!(r.mountains >= 0.65);
    }

    #[test]
    fn misma_descripcion_mismo_mundo() {
        let a = local_recipe("islas tropicales con ríos");
        let b = local_recipe("islas tropicales con ríos");
        assert_eq!(a.seed, b.seed);
        assert!(a.water_level >= 0.5 && a.rivers >= 0.6);
    }
}
