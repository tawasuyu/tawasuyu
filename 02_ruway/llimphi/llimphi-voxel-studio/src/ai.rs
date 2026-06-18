//! Conexión IA **"poor"** del studio: convierte una descripción en prosa en una
//! [`WorldRecipe`]. Dos caminos, en cascada:
//!
//! 1. **LLM** vía `pluma-llm` (`from_env`, autodetecta backend por env). Se le pide
//!    que emita la receta como literal RON; se parsea contra el struct real.
//! 2. **Heurística local** por palabras clave — instantánea y offline. Es el
//!    fallback cuando no hay modelo real (Mock) o la salida del LLM no parsea, así
//!    el botón **siempre** produce un mundo (de ahí lo "poor": sirve sin nube).

use llimphi_voxel::{Age, CharSpec, Clip, Flora, Material, SceneSpec, WorldRecipe};
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
/// literal parseable (lo usa el test).
#[cfg(test)]
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

/// **Escena desde prosa** (heurística local, instantánea/offline): deduce cuántos
/// actores y qué gesto del texto y arma la escena patrón "entran y saludan" en el
/// mundo `world`. `dim` es el tamaño del mundo (coords de grilla del guion).
pub fn generate_scene(prompt: &str, world: usize, dim: [u32; 3]) -> SceneSpec {
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
    let name = scene_name(prompt);
    SceneSpec::walk_and_emote(name, world, n, gesture, dim)
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

/// **Personaje desde prosa** (heurística local): deduce la edad y el color de la
/// remera de las palabras del texto; piel/pantalón quedan por defecto.
pub fn generate_character(prompt: &str) -> CharSpec {
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
    // Raíces (sin terminación de género/número) para casar "rojo/roja/rojas".
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
    let s: String = prompt.split_whitespace().take(4).collect::<Vec<_>>().join(" ");
    if s.is_empty() {
        "escena IA".into()
    } else {
        s
    }
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

    #[test]
    fn personaje_lee_edad_y_color() {
        let c = generate_character("un niño de remera roja");
        assert_eq!(c.age, Age::Child);
        assert_eq!(c.shirt, [0.82, 0.28, 0.26]);
    }

    #[test]
    fn escena_lee_cantidad_y_gesto() {
        let s = generate_scene("dos personajes que festejan", 1, [128, 56, 128]);
        assert_eq!(s.actors.len(), 2);
        assert_eq!(s.world, 1);
        // El gesto festejar entra como Cheer en la key del giro.
        let tiene_cheer = s.actors[0].keys.iter().any(|k| k.clip == Some(Clip::Cheer));
        assert!(tiene_cheer, "festejar → Cheer");
    }
}
