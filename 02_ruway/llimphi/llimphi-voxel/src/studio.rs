//! `studio` — el **modelo-documento del creador de mundos**: un [`Project`]
//! agnóstico de UI que junta los *artefactos* (mundos, personajes, escenas) bajo
//! nombre, para que **una interfaz** los cree/edite y la **IA** los emita o lea.
//!
//! Es contenido puro y **serializable** (RON para edición a mano / salida de la IA;
//! postcard para la CAS): no toca GPU ni ventana. La studio app (o cualquier otra)
//! lo carga, lo pinta con sus widgets y lo guarda. Cada artefacto referencia tipos
//! que ya existen en este crate ([`WorldRecipe`], [`Age`]) — el `Project` sólo les
//! pone nombre y los agrupa.

use serde::{Deserialize, Serialize};

use crate::actor::{Actor, Age, Clip};
use crate::director::{ActorKey, ActorScript};
use crate::worldgen::WorldRecipe;
use llimphi_3d::glam::Vec3;
use llimphi_3d::Camera3d;

/// Dimensión por defecto de la grilla con la que el editor previsualiza un mundo
/// (cúbica en XZ, alto = 0.4·lado, mínimo 48) — el mismo criterio que la app.
pub const PREVIEW_DIM_XZ: u32 = 128;

/// Calcula el `dim` `[x, y, z]` de un mundo de lado `xz` (alto derivado).
pub fn world_dim(xz: u32) -> [u32; 3] {
    let dy = (xz * 4 / 10).max(48);
    [xz, dy, xz]
}

/// Un **mundo nombrado** del proyecto: nombre + su [`WorldRecipe`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedWorld {
    pub name: String,
    pub recipe: WorldRecipe,
}

impl NamedWorld {
    pub fn new(name: impl Into<String>, recipe: WorldRecipe) -> Self {
        Self { name: name.into(), recipe }
    }
}

/// **Especificación serializable de un personaje**: lo que un editor/IA fija (edad
/// + colores). Se materializa con [`to_actor`](Self::to_actor) en un [`Actor`]
/// posable. Los colores son `[r, g, b]` en `[0,1]` (como [`Actor`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharSpec {
    pub name: String,
    pub age: Age,
    pub skin: [f32; 3],
    pub shirt: [f32; 3],
    pub pants: [f32; 3],
}

impl CharSpec {
    /// Un personaje con la paleta por defecto de [`Actor::new`] a la edad dada.
    pub fn new(name: impl Into<String>, age: Age) -> Self {
        let a = Actor::new(Vec3::ZERO, 0.0);
        Self { name: name.into(), age, skin: a.skin, shirt: a.shirt, pants: a.pants }
    }

    /// Materializa el spec en un [`Actor`] parado en `pos` mirando a `facing`.
    pub fn to_actor(&self, pos: Vec3, facing: f32) -> Actor {
        Actor::new(pos, facing)
            .with_age(self.age)
            .with_colors(self.skin, self.shirt, self.pants)
    }
}

/// **Keyframe serializable** de un actor (espejo de [`ActorKey`]): dónde está en
/// la grilla en `t`, y opcionalmente qué clip reproduce y hacia dónde mira.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ActorKeySpec {
    pub t: f32,
    pub gx: f32,
    pub gz: f32,
    #[serde(default)]
    pub clip: Option<Clip>,
    #[serde(default)]
    pub face: Option<f32>,
}

impl ActorKeySpec {
    /// Compila a un [`ActorKey`] del director.
    pub fn to_key(self) -> ActorKey {
        let mut k = ActorKey::at(self.t, self.gx, self.gz);
        if let Some(c) = self.clip {
            k = k.play(c);
        }
        if let Some(f) = self.face {
            k = k.facing(f);
        }
        k
    }
}

/// **Actor de una escena**: qué personaje del proyecto lo interpreta (`character`,
/// índice en [`Project::characters`]) y su guion de keyframes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActorSpec {
    pub character: usize,
    pub keys: Vec<ActorKeySpec>,
    /// **Tasa de cuadros propia** del actor (`None` = fluido/nativo). Con un valor
    /// bajo (12–15) el actor se anima *en doses* (stop-motion): es el sello que
    /// separa al Héroe del Avatar. Ver [`ActorScript::quantize`].
    #[serde(default)]
    pub frame_rate: Option<u32>,
}

impl ActorSpec {
    /// Compila los keyframes a un [`ActorScript`] reproducible (con su tasa de
    /// cuadros propia, si la tiene).
    pub fn to_script(&self) -> ActorScript {
        ActorScript::new(self.keys.iter().map(|k| k.to_key()).collect())
            .with_frame_rate(self.frame_rate)
    }
}

/// **Tipo de plano** de cámara: un encuadre cinematográfico de alto nivel que se
/// resuelve contra el **centroide del reparto** (no contra `eye/target` crudos), así
/// es trivial de elegir y de generar por IA. [`resolve`](Self::resolve) produce la
/// [`Camera3d`] del frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShotKind {
    /// Establecedor: lejos y alto, presenta la escena.
    Establishing,
    /// Primer plano: cerca, a la altura del pecho.
    CloseUp,
    /// Lateral: desde el costado.
    Side,
    /// Órbita: gira lento alrededor del reparto.
    Orbit,
}

impl ShotKind {
    /// Todos los planos (para ciclar en un editor).
    pub const ALL: [ShotKind; 4] =
        [ShotKind::Establishing, ShotKind::CloseUp, ShotKind::Side, ShotKind::Orbit];

    /// Nombre legible (español).
    pub fn label(self) -> &'static str {
        match self {
            ShotKind::Establishing => "establecedor",
            ShotKind::CloseUp => "primer plano",
            ShotKind::Side => "lateral",
            ShotKind::Orbit => "órbita",
        }
    }

    /// El plano siguiente (cicla).
    pub fn next(self) -> ShotKind {
        let i = ShotKind::ALL.iter().position(|&k| k == self).unwrap_or(0);
        ShotKind::ALL[(i + 1) % ShotKind::ALL.len()]
    }

    /// Resuelve la cámara del plano: mira a `look` (centroide del reparto, ya
    /// elevado a la altura del pecho), con el ojo según el tipo, a distancia base
    /// `d` (escala con el tamaño del reparto). `t` (seg) anima la órbita.
    pub fn resolve(self, look: Vec3, d: f32, t: f32) -> Camera3d {
        let (eye, fov) = match self {
            ShotKind::Establishing => {
                (look + Vec3::new(-0.5 * d, 0.9 * d, -1.6 * d), 50.0)
            }
            ShotKind::CloseUp => (look + Vec3::new(0.25 * d, 0.45 * d, -0.85 * d), 40.0),
            ShotKind::Side => (look + Vec3::new(1.35 * d, 0.4 * d, 0.15 * d), 46.0),
            ShotKind::Orbit => {
                let a = t * 0.6;
                (look + Vec3::new(a.cos() * 1.3 * d, 0.6 * d, a.sin() * 1.3 * d), 48.0)
            }
        };
        Camera3d { eye, target: look, fovy_rad: fov_f32_to_rad(fov), ..Camera3d::default() }
    }
}

/// Grados → radianes (helper local para no depender de glam en el call site).
fn fov_f32_to_rad(deg: f32) -> f32 {
    deg * std::f32::consts::PI / 180.0
}

/// Un **plano** de la escena: el tipo de encuadre y desde qué instante (seg) está
/// activo. El plano vigente en `t` es el último con `start ≤ t` (corte duro entre
/// planos).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ShotSpec {
    pub start: f32,
    pub kind: ShotKind,
}

/// **Especificación serializable de una escena**: el mundo de fondo (`world`,
/// índice en [`Project::worlds`]), la duración, el reparto guionado y los **planos**
/// de cámara. Es la versión editable/IA-emisible del [`Sequence`](crate::Sequence)
/// del director; se compila con [`scripts`](Self::scripts) y se reproduce posando
/// cada actor en `sample(t)` con la cámara del plano vigente.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneSpec {
    pub name: String,
    pub world: usize,
    pub duration: f32,
    pub actors: Vec<ActorSpec>,
    #[serde(default)]
    pub shots: Vec<ShotSpec>,
    /// **Cámara en mano**: intensidad del temblor orgánico (`0` = trípode fijo,
    /// look de dron; `~1` = respiración/pulso de camarógrafo). Ensucia la cámara
    /// matemáticamente perfecta del motor — es el sello que mete al espectador en
    /// el "barro" de la escena. Ver [`handheld_shake`].
    #[serde(default)]
    pub handheld: f32,
}

/// **Temblor de cámara en mano**, determinista (función pura de `t` → la peli sale
/// reproducible cuadro a cuadro). Suma de senos en frecuencias inconmensurables:
/// una **respiración** lenta (bob vertical) + un **micro-pulso** rápido en los tres
/// ejes para el ojo, y una **deriva** aún más lenta para el objetivo (el encuadre
/// flota, no sólo tiembla). `amt ≤ 0` → sin offset (trípode). Devuelve
/// `(offset_ojo, offset_objetivo)` en unidades de mundo, escalado un poco con la
/// distancia `d` del plano para que también respire en planos lejanos.
pub fn handheld_shake(t: f32, amt: f32, d: f32) -> (Vec3, Vec3) {
    if amt <= 0.0 {
        return (Vec3::ZERO, Vec3::ZERO);
    }
    let scale = amt * (1.0 + d * 0.03);
    // Respiración (bob) + micro-pulso por eje (fases y frecuencias dispares).
    let breath = (t * 1.7).sin() * 0.6 + (t * 0.9).sin() * 0.4;
    let jx = (t * 9.3).sin() * 0.5 + (t * 4.7 + 1.3).sin() * 0.5;
    let jy = (t * 8.1 + 2.1).sin() * 0.5 + (t * 5.3 + 0.7).sin() * 0.5;
    let jz = (t * 7.4 + 0.4).sin() * 0.5 + (t * 3.9 + 2.7).sin() * 0.5;
    let eye = Vec3::new(jx * 0.10, breath * 0.12 + jy * 0.08, jz * 0.10) * scale;
    // Deriva del objetivo: más lenta y desfasada → el cuadro "busca" al sujeto.
    let tgt = Vec3::new(
        (t * 1.3 + 0.5).sin() * 0.06,
        (t * 1.1 + 1.9).sin() * 0.05,
        0.0,
    ) * scale;
    (eye, tgt)
}

impl SceneSpec {
    /// Los guiones de los actores, listos para `sample(t)`.
    pub fn scripts(&self) -> Vec<ActorScript> {
        self.actors.iter().map(|a| a.to_script()).collect()
    }

    /// Los **instantes (seg) que merecen un acento musical**: los cortes de cámara
    /// (inicio de cada plano salvo el primero) y los **gestos** de los actores (keys
    /// con un clip *emote*). Es lo que deja caer la banda sonora *sobre la acción*.
    /// Ordenados, sin repetir (dos a menos de `EPS` se funden). Espeja
    /// [`Sequence::beat_times`](crate::Sequence::beat_times).
    pub fn beat_times(&self) -> Vec<f32> {
        const EPS: f32 = 0.05;
        let mut ts: Vec<f32> = Vec::new();
        for s in self.shots.iter().skip(1) {
            ts.push(s.start);
        }
        for a in &self.actors {
            for k in &a.keys {
                if k.clip.is_some_and(|c| c.is_emote()) {
                    ts.push(k.t);
                }
            }
        }
        ts.retain(|&t| t >= 0.0 && t <= self.duration + EPS);
        ts.sort_by(f32::total_cmp);
        ts.dedup_by(|a, b| (*a - *b).abs() < EPS);
        ts
    }

    /// La **cámara de la escena** en `t`: resuelve el plano vigente mirando a
    /// `look` (centroide del reparto) a distancia `d`, y le suma el temblor de
    /// **cámara en mano** ([`handheld_shake`]) según [`Self::handheld`]. Es el
    /// único punto por el que deberían pasar el preview y el export para que el
    /// sello de cámara salga igual en ambos.
    pub fn camera_at(&self, look: Vec3, d: f32, t: f32) -> Camera3d {
        let mut cam = self.active_shot(t).resolve(look, d, t);
        let (eo, to) = handheld_shake(t, self.handheld, d);
        cam.eye += eo;
        cam.target += to;
        cam
    }

    /// El plano vigente en `t` (el último con `start ≤ t`); `Establishing` si no
    /// hay planos definidos.
    pub fn active_shot(&self, t: f32) -> ShotKind {
        self.shots
            .iter()
            .filter(|s| s.start <= t)
            .last()
            .map(|s| s.kind)
            .unwrap_or(ShotKind::Establishing)
    }

    /// **Escena patrón "entran y saludan"**: `n` actores entran caminando por el
    /// centro del mundo de izquierda a derecha, se giran y hacen `gesture`. Coords
    /// de grilla (la altura del terreno se aplica al reproducir). La base tanto del
    /// arranque como de la generación por IA.
    pub fn walk_and_emote(
        name: impl Into<String>,
        world: usize,
        n: usize,
        gesture: Clip,
        dim: [u32; 3],
    ) -> Self {
        use std::f32::consts::{FRAC_PI_2, PI};
        let n = n.clamp(1, 5);
        let margin = 18.0_f32;
        let gx0 = margin;
        let gx1 = (dim[0] as f32 - margin).max(gx0 + 1.0);
        let cz = dim[2] as f32 * 0.5;
        let (t_walk, t_turn, dur) = (2.6_f32, 3.0_f32, 5.6_f32);

        let mut actors = Vec::with_capacity(n);
        for i in 0..n {
            let off = (i as f32 - (n as f32 - 1.0) / 2.0) * 3.0;
            let gz = cz + off;
            actors.push(ActorSpec {
                character: i,
                keys: vec![
                    ActorKeySpec { t: 0.0, gx: gx0, gz, clip: None, face: None },
                    ActorKeySpec { t: t_walk, gx: gx1, gz, clip: None, face: Some(FRAC_PI_2) },
                    ActorKeySpec { t: t_turn, gx: gx1, gz, clip: Some(gesture), face: Some(PI) },
                    ActorKeySpec { t: dur, gx: gx1, gz, clip: Some(gesture), face: Some(PI) },
                ],
                // El **Héroe** (primer actor) se anima en doses (12 fps): se mueve a
                // tirones, pesado, contra los demás (Avatares) que van fluidos. Es el
                // sello de animación, visible ya en la escena de arranque.
                frame_rate: if i == 0 { Some(12) } else { None },
            });
        }
        // Dos planos: establecedor durante la caminata, primer plano en el gesto.
        let shots = vec![
            ShotSpec { start: 0.0, kind: ShotKind::Establishing },
            ShotSpec { start: t_turn, kind: ShotKind::CloseUp },
        ];
        // Cámara en mano suave por defecto: el sello se ve sin tener que pedirlo.
        Self { name: name.into(), world, duration: dur, actors, shots, handheld: 0.7 }
    }
}

/// El **proyecto**: la bolsa de artefactos del creador (mundos, personajes,
/// escenas). Vacío por defecto; [`starter`](Self::starter) trae algo que tocar.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Project {
    pub worlds: Vec<NamedWorld>,
    #[serde(default)]
    pub characters: Vec<CharSpec>,
    #[serde(default)]
    pub scenes: Vec<SceneSpec>,
}

impl Project {
    /// Proyecto de arranque: el desierto y la pradera, un trío de personajes
    /// distinguibles y una escena demo (entran y saludan en el desierto).
    pub fn starter() -> Self {
        let characters = vec![
            CharSpec { name: "rojo".into(), age: Age::Adult, skin: [0.90, 0.72, 0.58], shirt: [0.82, 0.28, 0.26], pants: [0.20, 0.20, 0.28] },
            CharSpec { name: "azul".into(), age: Age::Adult, skin: [0.86, 0.68, 0.54], shirt: [0.22, 0.55, 0.78], pants: [0.18, 0.20, 0.24] },
            CharSpec { name: "amarillo".into(), age: Age::Adult, skin: [0.92, 0.78, 0.62], shirt: [0.92, 0.80, 0.30], pants: [0.26, 0.22, 0.20] },
        ];
        let dim = world_dim(PREVIEW_DIM_XZ);
        Self {
            worlds: vec![
                NamedWorld::new("desierto", WorldRecipe::desert(1337)),
                NamedWorld::new("pradera", WorldRecipe::grassland(1337)),
            ],
            characters,
            scenes: vec![SceneSpec::walk_and_emote("saludo en el desierto", 0, 3, Clip::Wave, dim)],
        }
    }

    /// Agrega un mundo y devuelve su índice.
    pub fn add_world(&mut self, w: NamedWorld) -> usize {
        self.worlds.push(w);
        self.worlds.len() - 1
    }

    /// Agrega una escena y devuelve su índice.
    pub fn add_scene(&mut self, s: SceneSpec) -> usize {
        self.scenes.push(s);
        self.scenes.len() - 1
    }

    /// Personaje `i`, o uno por defecto si el índice se sale (escenas que piden
    /// más actores que personajes hay).
    pub fn character_or_default(&self, i: usize) -> CharSpec {
        self.characters
            .get(i)
            .cloned()
            .unwrap_or_else(|| CharSpec::new("actor", Age::Adult))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proyecto_round_trip_ron() {
        let p = Project::starter();
        let s = ron::ser::to_string(&p).expect("serializa a ron");
        let back: Project = ron::from_str(&s).expect("deserializa de ron");
        assert_eq!(back.worlds.len(), p.worlds.len());
        assert_eq!(back.worlds[0].name, "desierto");
        // La receta sobrevive el viaje (un parámetro de muestra).
        assert!((back.worlds[0].recipe.base - p.worlds[0].recipe.base).abs() < 1e-6);
    }

    #[test]
    fn charspec_se_materializa_con_la_edad() {
        let spec = CharSpec::new("nene", Age::Baby);
        let actor = spec.to_actor(Vec3::ZERO, 0.0);
        assert_eq!(actor.age, Age::Baby);
    }

    #[test]
    fn world_dim_minimo_48_de_alto() {
        assert_eq!(world_dim(64)[1], 48); // 64*0.4=25.6 → clamp a 48
        assert_eq!(world_dim(192)[1], 76);
    }

    #[test]
    fn escena_round_trip_y_compila_a_guiones() {
        let dim = world_dim(128);
        let s = SceneSpec::walk_and_emote("demo", 0, 3, Clip::Wave, dim);
        // RON ida y vuelta.
        let txt = ron::ser::to_string(&s).expect("ron");
        let back: SceneSpec = ron::from_str(&txt).expect("de-ron");
        assert_eq!(back.actors.len(), 3);
        // Compila a guiones reproducibles: a mitad de la caminata el actor se movió.
        let scripts = back.scripts();
        let start = scripts[0].sample(0.0);
        let mid = scripts[0].sample(1.3);
        assert!(mid.gx > start.gx, "el actor avanza en X: {} → {}", start.gx, mid.gx);
    }

    #[test]
    fn plano_vigente_corta_en_el_tiempo() {
        let dim = world_dim(128);
        let s = SceneSpec::walk_and_emote("demo", 0, 2, Clip::Wave, dim);
        // Arranca en establecedor; tras el giro (t≈3) pasa a primer plano.
        assert_eq!(s.active_shot(0.5), ShotKind::Establishing);
        assert_eq!(s.active_shot(3.5), ShotKind::CloseUp);
        // El plano resuelve una cámara que mira al centroide.
        let look = Vec3::new(10.0, 2.0, 10.0);
        let cam = ShotKind::CloseUp.resolve(look, 9.0, 1.0);
        assert_eq!(cam.target, look);
        assert!((cam.eye - look).length() > 1.0, "el ojo está separado del objetivo");
    }

    #[test]
    fn beats_son_cortes_y_gestos() {
        let dim = world_dim(128);
        let s = SceneSpec::walk_and_emote("demo", 0, 2, Clip::Wave, dim);
        // walk_and_emote: corte de cámara a t_turn (3.0) + el gesto Wave a t_turn.
        // Caen en el mismo instante → se funden en un solo beat.
        let beats = s.beat_times();
        assert!(!beats.is_empty(), "hay al menos un acento");
        assert!(beats.iter().all(|&t| t >= 0.0 && t <= s.duration + 0.1));
        assert!(beats.iter().any(|&t| (t - 3.0).abs() < 0.2), "acento cerca del gesto/corte");
    }

    #[test]
    fn camara_en_mano_es_determinista_y_apagable() {
        // amt=0 → trípode: sin offset, exactamente cero.
        let (e0, t0) = handheld_shake(1.234, 0.0, 30.0);
        assert_eq!(e0, Vec3::ZERO);
        assert_eq!(t0, Vec3::ZERO);

        // amt>0 → tiembla (offset no nulo) y es función pura de t (reproducible).
        let (e1, _) = handheld_shake(1.234, 0.7, 30.0);
        let (e2, _) = handheld_shake(1.234, 0.7, 30.0);
        assert_eq!(e1, e2, "mismo t → mismo temblor (peli reproducible)");
        assert!(e1.length() > 0.0, "con intensidad la cámara se mueve");
        // Instantes distintos → temblor distinto (no está congelado).
        let (e3, _) = handheld_shake(1.235, 0.7, 30.0);
        assert!((e1 - e3).length() > 0.0, "el temblor evoluciona en el tiempo");
    }

    #[test]
    fn frame_rate_del_heroe_viaja_al_guion() {
        let dim = world_dim(128);
        let s = SceneSpec::walk_and_emote("demo", 0, 3, Clip::Wave, dim);
        // El primer actor (Héroe) anima en doses; los demás, fluidos.
        assert_eq!(s.actors[0].frame_rate, Some(12));
        assert_eq!(s.actors[1].frame_rate, None);
        // Y sobrevive la compilación a guion.
        assert_eq!(s.scripts()[0].frame_rate(), Some(12));
        // Cámara en mano por defecto encendida.
        assert!(s.handheld > 0.0);
    }

    #[test]
    fn starter_trae_escena_y_personajes() {
        let p = Project::starter();
        assert_eq!(p.characters.len(), 3);
        assert_eq!(p.scenes.len(), 1);
        assert_eq!(p.scenes[0].world, 0);
    }
}
