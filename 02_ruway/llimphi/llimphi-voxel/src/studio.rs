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
use crate::rig::{Andar, Rig, RigPose};
use crate::worldgen::{Bioma, BiomaPalette, Forma, Material, ResolvedMaterial};
use llimphi_3d::glam::{Mat4, Vec3};
use llimphi_3d::{Camera3d, Vertex3d};

/// Dimensión por defecto de la grilla con la que el editor previsualiza un mundo
/// (cúbica en XZ, alto = 0.4·lado, mínimo 48) — el mismo criterio que la app.
pub const PREVIEW_DIM_XZ: u32 = 128;

/// **Dirección del sol** de la escena (hacia el sol, sin normalizar). Una sola
/// fuente de verdad: la usa el preview/export para iluminar (`voxel.sun_dir`) y el
/// plano [`ShotKind::Backlight`] para saber adónde mirar. Sol algo bajo → luz
/// rasante (claroscuro) y, en contraluz, **god rays** legibles.
pub const SCENE_SUN: [f32; 3] = [0.5, 0.36, 0.45];

/// Calcula el `dim` `[x, y, z]` de un mundo de lado `xz` (alto derivado).
pub fn world_dim(xz: u32) -> [u32; 3] {
    let dy = (xz * 4 / 10).max(48);
    [xz, dy, xz]
}

/// **Paso de reubicación** de la ventana de una escena (voxels). El origen sólo
/// salta a múltiplos de este valor: así la ventana no se regenera cada cuadro al
/// caminar el reparto, sólo al cruzar un paso. Como la cámara mira al centroide del
/// reparto (en espacio de ventana), el salto traslada terreno + cámara + actores
/// **juntos** → sin pop visible (la imagen es continua), pero el mundo no tiene
/// borde: el reparto puede caminar indefinidamente. Ver [`window_origin_for_cast`].
pub const SCENE_WINDOW_STEP: i32 = 16;

/// Redondea `v` al múltiplo de `step` hacia abajo (floor con signo) — espeja el
/// `snap` del [`WorldStream`](crate::WorldStream).
#[inline]
fn snap(v: i32, step: i32) -> i32 {
    v.div_euclid(step) * step
}

/// **Origen de ventana (esquina, coords de mundo) que centra al reparto** de una
/// escena en el instante `t`: promedia las columnas `(gx, gz)` de los guiones
/// (tomadas en su tiempo cuantizado, igual que al posar) y resta medio `dim`,
/// snappeado a [`SCENE_WINDOW_STEP`]. Es lo que hace que **toda escena viva en un
/// mundo infinito**: la ventana voxel sigue al reparto en vez de ser una caja fija
/// centrada en el origen. Sin reparto, devuelve `[0, 0]` (mundo centrado, como
/// antes).
pub fn window_origin_for_cast(scripts: &[ActorScript], t: f32, dim: [u32; 3]) -> [i32; 2] {
    if scripts.is_empty() {
        return [0, 0];
    }
    let (mut cwx, mut cwz) = (0.0_f32, 0.0_f32);
    for s in scripts {
        let smp = s.sample(s.quantize(t));
        cwx += smp.gx;
        cwz += smp.gz;
    }
    let n = scripts.len() as f32;
    let (cwx, cwz) = (cwx / n, cwz / n);
    [
        snap(cwx as i32 - dim[0] as i32 / 2, SCENE_WINDOW_STEP),
        snap(cwz as i32 - dim[2] as i32 / 2, SCENE_WINDOW_STEP),
    ]
}

// =============================================================================
//  Nivel 1 — Leyes (físicas / comportamientos)
// =============================================================================

/// **Tipo de ley** (comportamiento matemático). Enum extensible: hoy `Fluir`
/// (líquidos), `Crecer` (flora) y `Custom` (placeholder, para abrir a hechizos sin
/// codear su runtime todavía). **No se simula aún** — es un spec declarativo que un
/// material adopta y parametriza. Cada variante expone sus parámetros editables vía
/// [`params`](Self::params) para que la UI arme sliders sin conocer cada caso.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LeyKind {
    /// Líquido: se esparce a vecinos en horizontal y cae con gravedad (cascadas).
    Fluir { gravedad: f32, horizontal: f32 },
    /// Crece a una velocidad VARIABLE (flora).
    Crecer { velocidad: f32 },
    /// Comportamiento a definir (arquitectura abierta; sin parámetros aún).
    Custom,
}

impl LeyKind {
    /// Un ejemplar de cada tipo, con valores por defecto (para crear/ciclar).
    pub fn all_defaults() -> Vec<LeyKind> {
        vec![
            LeyKind::Fluir { gravedad: 1.0, horizontal: 0.6 },
            LeyKind::Crecer { velocidad: 1.0 },
            LeyKind::Custom,
        ]
    }

    /// Nombre legible (español).
    pub fn label(&self) -> &'static str {
        match self {
            LeyKind::Fluir { .. } => "fluir",
            LeyKind::Crecer { .. } => "crecer",
            LeyKind::Custom => "custom",
        }
    }

    /// El tipo siguiente (cicla por los defaults), preservando nada (resetea params).
    pub fn next(&self) -> LeyKind {
        let all = Self::all_defaults();
        let i = all.iter().position(|k| k.label() == self.label()).unwrap_or(0);
        all[(i + 1) % all.len()].clone()
    }

    /// Parámetros editables: `(nombre, valor, min, max)`. La UI arma un slider por cada.
    pub fn params(&self) -> Vec<(&'static str, f32, f32, f32)> {
        match self {
            LeyKind::Fluir { gravedad, horizontal } => vec![
                ("gravedad", *gravedad, 0.0, 1.0),
                ("horizontal", *horizontal, 0.0, 1.0),
            ],
            LeyKind::Crecer { velocidad } => vec![("velocidad", *velocidad, 0.0, 5.0)],
            LeyKind::Custom => vec![],
        }
    }

    /// Fija el parámetro `i` (en el orden de [`params`](Self::params)) a `v`.
    pub fn set_param(&mut self, i: usize, v: f32) {
        match self {
            LeyKind::Fluir { gravedad, horizontal } => match i {
                0 => *gravedad = v,
                1 => *horizontal = v,
                _ => {}
            },
            LeyKind::Crecer { velocidad } => {
                if i == 0 {
                    *velocidad = v;
                }
            }
            LeyKind::Custom => {}
        }
    }
}

/// Una **Ley**: nombre + tipo (con sus parámetros). El nivel más básico de la
/// composición; los materiales la adoptan vía [`LeyUso`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ley {
    pub id: u64,
    pub name: String,
    pub kind: LeyKind,
}

// =============================================================================
//  Nivel 2 — Materiales
// =============================================================================

/// **Rol de un material**: relleno de terreno o un objeto colocable (con su forma).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatRole {
    /// Relleno del terreno (suelo, acantilado, cumbre…).
    Terreno,
    /// Objeto colocado sobre el suelo (cactus, árbol, roca suelta…).
    Objeto(Forma),
}

impl MatRole {
    pub fn label(&self) -> &'static str {
        match self {
            MatRole::Terreno => "terreno",
            MatRole::Objeto(_) => "objeto",
        }
    }
}

/// **Uso de una ley por un material**: a qué ley refiere y con qué parámetros
/// concretos (overridean los defaults de la ley — «espesor del líquido», etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeyUso {
    pub ley: u64,
    pub params: Vec<f32>,
}

/// Un **material autorable**: color/grano (texturas a futuro) + rol + leyes
/// aplicadas, con **herencia viva por padre**. Los campos visuales son `Option`:
/// `None` = heredar del padre (o del builtin). [`Project::resolve_material`] aplana
/// la cadena. Así «cactus amarillo» = hijo de «cactus verde» que sólo redefine el
/// color.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialDef {
    pub id: u64,
    pub name: String,
    /// Material del que hereda lo no redefinido (`None` = raíz).
    #[serde(default)]
    pub parent: Option<u64>,
    pub role: MatRole,
    /// Color `[r,g,b]` en `[0,1]`; `None` = heredar.
    #[serde(default)]
    pub color: Option<[f32; 3]>,
    /// Grano de materia `[0,1]`; `None` = heredar.
    #[serde(default)]
    pub grain: Option<f32>,
    /// Leyes que adopta, con sus parámetros.
    #[serde(default)]
    pub leyes: Vec<LeyUso>,
    /// Tag de paleta semilla (de fábrica): deja que la IA/preset encuentre «la
    /// arena» sin clavar un id. `None` en materiales hechos por el usuario.
    #[serde(default)]
    pub builtin: Option<Material>,
}

impl MaterialDef {
    /// Material de fábrica a partir de una variante semilla (color/grano/rol).
    pub fn from_builtin(id: u64, m: Material) -> Self {
        let role = match m {
            Material::Cactus => MatRole::Objeto(Forma::Columnar),
            _ => MatRole::Terreno,
        };
        let c = m.color();
        Self {
            id,
            name: m.label().to_string(),
            parent: None,
            role,
            color: Some([c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0]),
            grain: Some(m.grain()),
            leyes: Vec::new(),
            builtin: Some(m),
        }
    }
}

// =============================================================================
//  Nivel 5 — Mundos
// =============================================================================

/// Un **Mundo**: semilla + uno o más biomas. La semilla es lo único «numérico» que
/// el usuario teclea (con botón de random en la UI). El stage 1 genera con el primer
/// bioma; la distribución multi-bioma queda como refinamiento.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mundo {
    pub id: u64,
    pub name: String,
    pub seed: u32,
    #[serde(default)]
    pub biomas: Vec<u64>,
}

/// Lo que el render necesita de un mundo: el bioma (relieve), la semilla y la paleta
/// **ya resuelta** (colores concretos). Lo arma [`Project::render_mundo`].
#[derive(Debug, Clone)]
pub struct MundoRender {
    pub bioma: Bioma,
    pub seed: u32,
    pub palette: BiomaPalette,
}

/// **Especificación serializable de un personaje**: lo que un editor/IA fija (edad
/// + colores). Se materializa con [`to_actor`](Self::to_actor) en un [`Actor`]
/// posable. Los colores son `[r, g, b]` en `[0,1]` (como [`Actor`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharSpec {
    /// Id estable (asignado por el [`Project`]); `0` hasta que se agrega.
    #[serde(default)]
    pub id: u64,
    pub name: String,
    pub age: Age,
    pub skin: [f32; 3],
    pub shirt: [f32; 3],
    pub pants: [f32; 3],
    /// **Cuerpo**: `None` = humanoide (muñeco [`Actor`] rico: cara, gestos, cross-fade,
    /// look-at); `Some(rig)` = una morfología arbitraria ([`Rig`]: cuadrúpedo, ave,
    /// serpiente…) animada por su [`Andar`]. Default `None` (compatibilidad).
    #[serde(default)]
    pub rig: Option<Rig>,
}

impl CharSpec {
    /// Un personaje con la paleta por defecto de [`Actor::new`] a la edad dada.
    /// `id = 0` hasta que el [`Project`] se lo asigna al agregarlo.
    pub fn new(name: impl Into<String>, age: Age) -> Self {
        let a = Actor::new(Vec3::ZERO, 0.0);
        Self { id: 0, name: name.into(), age, skin: a.skin, shirt: a.shirt, pants: a.pants, rig: None }
    }

    /// Nombre legible del cuerpo (para la UI).
    pub fn cuerpo_label(&self) -> &str {
        match &self.rig {
            None => "humanoide",
            Some(r) => r.nombre.as_str(),
        }
    }

    /// Cicla el cuerpo: humanoide → cuadrúpedo → ave → serpiente → humanoide.
    pub fn cycle_cuerpo(&mut self) {
        let presets = Rig::presets();
        self.rig = match &self.rig {
            None => Some(presets[1].clone()), // humanoide(None) → cuadrúpedo
            Some(r) => {
                let i = presets.iter().position(|p| p.nombre == r.nombre).unwrap_or(0);
                if i + 1 < presets.len() {
                    Some(presets[i + 1].clone())
                } else {
                    None // serpiente → humanoide
                }
            }
        };
    }

    /// **Meta de render** del ser en `pos`/`facing` para una animación: la matriz de
    /// modelo + la malla. Humanoide (`rig = None`) usa el [`Actor`] rico (clip, fase,
    /// look-at); un rig usa su [`Andar::caminar`] al andar (Walk/Run) o pose neutra si
    /// no. Centraliza el branch para el preview y las escenas.
    pub fn to_meta(
        &self,
        pos: Vec3,
        facing: f32,
        clip: Clip,
        phase: f32,
        look: Option<Vec3>,
    ) -> (Mat4, Vec<Vertex3d>, Vec<u16>) {
        match &self.rig {
            None => {
                let mut a = self.to_actor(pos, facing);
                a.set_clip(clip);
                a.advance(phase);
                a.look_at(look);
                let (v, i) = a.mesh();
                (a.model(), v, i)
            }
            Some(rig) => {
                let moving = matches!(clip, Clip::Walk | Clip::Run);
                let pose = if moving {
                    Andar::caminar(rig).pose(phase)
                } else {
                    RigPose::neutra(rig.len())
                };
                let (v, i) = rig.mesh(&pose, self.skin, self.shirt, self.pants);
                let model = Mat4::from_translation(pos) * Mat4::from_rotation_y(facing);
                (model, v, i)
            }
        }
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
    /// **Contraluz**: cámara del lado opuesto al sol, mirando hacia él con el reparto
    /// a contraluz (silueta en el tercio bajo) y el disco solar en lo alto del cuadro
    /// → rim light + **god rays** (haces volumétricos). El plano "héroe" del motor.
    Backlight,
}

impl ShotKind {
    /// Todos los planos (para ciclar en un editor).
    pub const ALL: [ShotKind; 5] = [
        ShotKind::Establishing,
        ShotKind::CloseUp,
        ShotKind::Side,
        ShotKind::Orbit,
        ShotKind::Backlight,
    ];

    /// Nombre legible (español).
    pub fn label(self) -> &'static str {
        match self {
            ShotKind::Establishing => "establecedor",
            ShotKind::CloseUp => "primer plano",
            ShotKind::Side => "lateral",
            ShotKind::Orbit => "órbita",
            ShotKind::Backlight => "contraluz",
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
        // `(eye, fov, target)` por tipo. Casi todos miran al reparto (`look`); el
        // contraluz sube el objetivo hacia el sol para meterlo en el cuadro.
        let (eye, fov, target) = match self {
            ShotKind::Establishing => {
                (look + Vec3::new(-0.5 * d, 0.9 * d, -1.6 * d), 50.0, look)
            }
            ShotKind::CloseUp => (look + Vec3::new(0.25 * d, 0.45 * d, -0.85 * d), 40.0, look),
            ShotKind::Side => (look + Vec3::new(1.35 * d, 0.4 * d, 0.15 * d), 46.0, look),
            ShotKind::Orbit => {
                let a = t * 0.6;
                (look + Vec3::new(a.cos() * 1.3 * d, 0.6 * d, a.sin() * 1.3 * d), 48.0, look)
            }
            ShotKind::Backlight => {
                // Detrás del reparto respecto al sol (horizontal anti-sol), apenas
                // elevado; el objetivo sube hacia el cielo → la cámara mira hacia el
                // sol con el reparto a contraluz en el tercio bajo.
                let s = Vec3::new(SCENE_SUN[0], SCENE_SUN[1], SCENE_SUN[2]).normalize();
                let back = Vec3::new(-s.x, 0.0, -s.z).normalize();
                let eye = look + back * (1.2 * d) + Vec3::new(0.0, 0.15 * d, 0.0);
                let target = look + Vec3::new(0.0, 0.55 * d, 0.0);
                (eye, 55.0, target)
            }
        };
        Camera3d { eye, target, fovy_rad: fov_f32_to_rad(fov), ..Camera3d::default() }
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
    /// Id estable (asignado por el [`Project`]).
    #[serde(default)]
    pub id: u64,
    pub name: String,
    /// El **mundo** de fondo (id en [`Project::mundos`]).
    pub mundo: u64,
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
        mundo: u64,
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
        // Tres planos: establecedor, **contraluz** a mitad de la caminata (el reparto
        // entra al sol → luce los god rays), y primer plano en el gesto.
        let shots = vec![
            ShotSpec { start: 0.0, kind: ShotKind::Establishing },
            ShotSpec { start: t_walk * 0.5, kind: ShotKind::Backlight },
            ShotSpec { start: t_turn, kind: ShotKind::CloseUp },
        ];
        // Cámara en mano suave por defecto: el sello se ve sin tener que pedirlo.
        Self { id: 0, name: name.into(), mundo, duration: dur, actors, shots, handheld: 0.7 }
    }
}

/// El **proyecto**: la bolsa de artefactos del creador, por niveles de composición
/// (leyes → materiales → seres → biomas → mundos → escenas). Cada item lleva un `id`
/// estable (separado del nombre) para referenciarse, renombrarse y duplicarse.
/// Vacío por defecto; [`starter`](Self::starter) trae algo que tocar.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Project {
    /// Contador de ids; `0` queda reservado como «ninguno».
    #[serde(default)]
    pub next_id: u64,
    #[serde(default)]
    pub leyes: Vec<Ley>,
    #[serde(default)]
    pub materiales: Vec<MaterialDef>,
    /// Los **seres** (personajes); el tipo se llama `CharSpec` por historia.
    #[serde(default)]
    pub seres: Vec<CharSpec>,
    #[serde(default)]
    pub biomas: Vec<Bioma>,
    #[serde(default)]
    pub mundos: Vec<Mundo>,
    #[serde(default)]
    pub escenas: Vec<SceneSpec>,
}

impl Project {
    /// Reserva el próximo id estable.
    pub fn alloc_id(&mut self) -> u64 {
        if self.next_id == 0 {
            self.next_id = 1;
        }
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Proyecto de arranque: leyes base, los 6 materiales semilla, un trío de seres,
    /// los biomas desierto/pradera y dos mundos que los envuelven, más la escena demo.
    pub fn starter() -> Self {
        let mut p = Project { next_id: 1, ..Default::default() };

        // --- Leyes base ---
        let ley_fluir = p.alloc_id();
        p.leyes.push(Ley {
            id: ley_fluir,
            name: "fluir".into(),
            kind: LeyKind::Fluir { gravedad: 1.0, horizontal: 0.6 },
        });
        let ley_crecer = p.alloc_id();
        p.leyes.push(Ley {
            id: ley_crecer,
            name: "crecer".into(),
            kind: LeyKind::Crecer { velocidad: 1.0 },
        });

        // --- Materiales semilla (arena/pasto/roca/nieve/agua/cactus) ---
        let mut mat_ids = std::collections::HashMap::new();
        for m in Material::ALL {
            let id = p.alloc_id();
            let mut def = MaterialDef::from_builtin(id, m);
            // El agua fluye; el cactus crece — leyes de muestra (sin simular aún).
            match m {
                Material::Water => def.leyes.push(LeyUso { ley: ley_fluir, params: vec![1.0, 0.6] }),
                Material::Cactus => def.leyes.push(LeyUso { ley: ley_crecer, params: vec![1.0] }),
                _ => {}
            }
            mat_ids.insert(m, id);
            p.materiales.push(def);
        }
        let mid = |m: Material| mat_ids[&m];

        // --- Seres ---
        for (name, skin, shirt, pants) in [
            ("rojo", [0.90, 0.72, 0.58], [0.82, 0.28, 0.26], [0.20, 0.20, 0.28]),
            ("azul", [0.86, 0.68, 0.54], [0.22, 0.55, 0.78], [0.18, 0.20, 0.24]),
            ("amarillo", [0.92, 0.78, 0.62], [0.92, 0.80, 0.30], [0.26, 0.22, 0.20]),
        ] {
            let id = p.alloc_id();
            p.seres.push(CharSpec { id, name: name.into(), age: Age::Adult, skin, shirt, pants, rig: None });
        }

        // --- Biomas ---
        let bioma_desierto = p.alloc_id();
        p.biomas.push(Bioma {
            id: bioma_desierto,
            name: "desierto".into(),
            base: 0.30,
            dune: 0.05,
            relief: 0.45,
            mountains: 0.12,
            water_level: 0.26,
            rivers: 0.18,
            peak_at: 1.0,
            ground: mid(Material::Sand),
            cliff: mid(Material::Rock),
            peak: None,
            objetos: vec![crate::worldgen::ObjetoUso {
                material: mid(Material::Cactus),
                densidad: 0.010,
                forma: Forma::Columnar,
            }],
            seres: vec![],
        });
        let bioma_pradera = p.alloc_id();
        p.biomas.push(Bioma {
            id: bioma_pradera,
            name: "pradera".into(),
            base: 0.22,
            dune: 0.10,
            relief: 0.7,
            mountains: 0.5,
            water_level: 0.30,
            rivers: 0.25,
            peak_at: 0.80,
            ground: mid(Material::Grass),
            cliff: mid(Material::Rock),
            peak: Some(mid(Material::Snow)),
            objetos: vec![],
            seres: vec![],
        });

        // --- Mundos ---
        let mundo_desierto = p.alloc_id();
        p.mundos.push(Mundo {
            id: mundo_desierto,
            name: "desierto".into(),
            seed: 1337,
            biomas: vec![bioma_desierto],
        });
        let mundo_pradera = p.alloc_id();
        p.mundos.push(Mundo {
            id: mundo_pradera,
            name: "pradera".into(),
            seed: 1337,
            biomas: vec![bioma_pradera],
        });

        // --- Escena demo ---
        let dim = world_dim(PREVIEW_DIM_XZ);
        let mut escena = SceneSpec::walk_and_emote("saludo en el desierto", mundo_desierto, 3, Clip::Wave, dim);
        escena.id = p.alloc_id();
        p.escenas.push(escena);

        p
    }

    // --- Búsquedas por id ---
    pub fn ley(&self, id: u64) -> Option<&Ley> {
        self.leyes.iter().find(|x| x.id == id)
    }
    pub fn material(&self, id: u64) -> Option<&MaterialDef> {
        self.materiales.iter().find(|x| x.id == id)
    }
    pub fn bioma(&self, id: u64) -> Option<&Bioma> {
        self.biomas.iter().find(|x| x.id == id)
    }
    pub fn mundo(&self, id: u64) -> Option<&Mundo> {
        self.mundos.iter().find(|x| x.id == id)
    }

    /// Id del material de fábrica de la variante `m`, si está sembrado (para que la
    /// IA/presets encuentren «la arena» sin clavar números).
    pub fn material_id_for(&self, m: Material) -> Option<u64> {
        self.materiales.iter().find(|d| d.builtin == Some(m)).map(|d| d.id)
    }

    /// **Resuelve un material** aplanando la cadena de herencia: el primer `Some`
    /// hacia arriba gana; si nada lo define, cae al color/grano del `builtin`, y por
    /// último a un gris neutro. Corta a profundidad 32 (anti-ciclo).
    pub fn resolve_material(&self, id: u64) -> ResolvedMaterial {
        let mut color: Option<[f32; 3]> = None;
        let mut grain: Option<f32> = None;
        let mut builtin: Option<Material> = None;
        let mut cur = Some(id);
        let mut depth = 0;
        while let Some(cid) = cur {
            if depth > 32 {
                break;
            }
            let Some(m) = self.material(cid) else { break };
            if color.is_none() {
                color = m.color;
            }
            if grain.is_none() {
                grain = m.grain;
            }
            if builtin.is_none() {
                builtin = m.builtin;
            }
            cur = m.parent;
            depth += 1;
        }
        let color_u8 = color
            .map(|c| {
                [
                    (c[0].clamp(0.0, 1.0) * 255.0) as u8,
                    (c[1].clamp(0.0, 1.0) * 255.0) as u8,
                    (c[2].clamp(0.0, 1.0) * 255.0) as u8,
                ]
            })
            .or_else(|| builtin.map(|b| b.color()))
            .unwrap_or([150, 150, 150]);
        let grain = grain.or_else(|| builtin.map(|b| b.grain())).unwrap_or(0.0);
        ResolvedMaterial::new(color_u8, grain)
    }

    /// **Leyes efectivas** de un material: las propias + las heredadas de sus padres,
    /// cada una con sus parámetros aplicados (la `LeyUso.params` overridea los
    /// defaults de la ley). Es lo que dispara la simulación: un material *fluye*
    /// porque tiene una [`LeyKind::Fluir`], *crece* porque tiene [`LeyKind::Crecer`].
    pub fn resolve_laws(&self, material_id: u64) -> Vec<LeyKind> {
        let mut out = Vec::new();
        let mut cur = Some(material_id);
        let mut depth = 0;
        while let Some(cid) = cur {
            if depth > 32 {
                break;
            }
            let Some(m) = self.material(cid) else { break };
            for u in &m.leyes {
                if let Some(ley) = self.ley(u.ley) {
                    let mut kind = ley.kind.clone();
                    for (i, &v) in u.params.iter().enumerate() {
                        kind.set_param(i, v);
                    }
                    out.push(kind);
                }
            }
            cur = m.parent;
            depth += 1;
        }
        out
    }

    /// Si el material tiene una ley [`Fluir`](LeyKind::Fluir), sus parámetros
    /// `(gravedad, horizontal)` — lo que vuelve líquido a un material. `None` = no fluye.
    pub fn fluir_params(&self, material_id: u64) -> Option<(f32, f32)> {
        self.resolve_laws(material_id).into_iter().find_map(|k| match k {
            LeyKind::Fluir { gravedad, horizontal } => Some((gravedad, horizontal)),
            _ => None,
        })
    }

    /// Si el material tiene una ley [`Crecer`](LeyKind::Crecer), su `velocidad`.
    /// `None` = no crece.
    pub fn crecer_velocidad(&self, material_id: u64) -> Option<f32> {
        self.resolve_laws(material_id).into_iter().find_map(|k| match k {
            LeyKind::Crecer { velocidad } => Some(velocidad),
            _ => None,
        })
    }

    /// Parámetros de Fluir del **agua** (el material semilla `Water`), si los tiene.
    /// El studio lo usa para que el agua del bioma fluya según su ley.
    pub fn water_fluir(&self) -> Option<(f32, f32)> {
        self.fluir_params(self.material_id_for(Material::Water)?)
    }

    /// Arma la [`BiomaPalette`] resuelta de un bioma (colores concretos para el render).
    pub fn bioma_palette(&self, b: &Bioma) -> BiomaPalette {
        let agua = self
            .material_id_for(Material::Water)
            .map(|id| self.resolve_material(id).color)
            .unwrap_or_else(|| Material::Water.color());
        BiomaPalette {
            ground: self.resolve_material(b.ground),
            cliff: self.resolve_material(b.cliff),
            peak: b.peak.map(|id| self.resolve_material(id)),
            agua,
            objetos: b
                .objetos
                .iter()
                .map(|o| (self.resolve_material(o.material), o.densidad, o.forma))
                .collect(),
        }
    }

    /// Todo lo que el render necesita de un mundo: su primer bioma (relieve), la
    /// semilla y la paleta resuelta. `None` si el mundo o su bioma no existen.
    pub fn render_mundo(&self, mundo_id: u64) -> Option<MundoRender> {
        let m = self.mundo(mundo_id)?;
        let bid = *m.biomas.first()?;
        let bioma = self.bioma(bid)?.clone();
        let palette = self.bioma_palette(&bioma);
        Some(MundoRender { bioma, seed: m.seed, palette })
    }

    /// El render de un **bioma** suelto (modo Biomas), con una semilla fija de
    /// previsualización. `None` si el bioma no existe.
    pub fn render_bioma(&self, bioma_id: u64) -> Option<MundoRender> {
        let bioma = self.bioma(bioma_id)?.clone();
        let palette = self.bioma_palette(&bioma);
        Some(MundoRender { bioma, seed: 1337, palette })
    }

    /// Ser `i` (por índice, como referencian los actores de una escena), o uno por
    /// defecto si el índice se sale.
    pub fn character_or_default(&self, i: usize) -> CharSpec {
        self.seres
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
        assert_eq!(back.biomas.len(), p.biomas.len());
        assert_eq!(back.mundos.len(), p.mundos.len());
        assert_eq!(back.biomas[0].name, "desierto");
        // El relieve del bioma sobrevive el viaje (un parámetro de muestra).
        assert!((back.biomas[0].base - p.biomas[0].base).abs() < 1e-6);
        // Y los ids siguen apuntando: la escena referencia un mundo que existe.
        assert!(back.mundo(back.escenas[0].mundo).is_some());
    }

    #[test]
    fn herencia_de_material_overridea_solo_lo_redefinido() {
        let mut p = Project::starter();
        // «cactus amarillo» hijo de «cactus»: sólo redefine el color.
        let cactus = p.material_id_for(Material::Cactus).unwrap();
        let verde = p.resolve_material(cactus);
        let hijo_id = p.alloc_id();
        p.materiales.push(MaterialDef {
            id: hijo_id,
            name: "cactus amarillo".into(),
            parent: Some(cactus),
            role: MatRole::Objeto(Forma::Columnar),
            color: Some([0.9, 0.85, 0.2]),
            grain: None, // hereda
            leyes: vec![],
            builtin: None,
        });
        let hijo = p.resolve_material(hijo_id);
        assert_ne!(hijo.color, verde.color, "el color se redefinió");
        assert_eq!(hijo.grain, verde.grain, "el grano se heredó del padre");
    }

    #[test]
    fn agua_fluye_por_su_ley_y_hereda_leyes() {
        let p = Project::starter();
        // El agua semilla tiene la ley Fluir con sus params.
        let (g, h) = p.water_fluir().expect("el agua tiene ley Fluir");
        assert!(g > 0.0 && h >= 0.0);
        // Un material sin leyes no fluye.
        let roca = p.material_id_for(Material::Rock).unwrap();
        assert!(p.fluir_params(roca).is_none());
        // El cactus crece (ley Crecer en el starter).
        let cactus = p.material_id_for(Material::Cactus).unwrap();
        assert!(p.crecer_velocidad(cactus).is_some());

        // Herencia de leyes: un hijo del agua hereda Fluir aunque no la redefina.
        let mut p2 = p.clone();
        let agua = p2.material_id_for(Material::Water).unwrap();
        let hijo = p2.alloc_id();
        p2.materiales.push(MaterialDef {
            id: hijo,
            name: "agua turbia".into(),
            parent: Some(agua),
            role: MatRole::Terreno,
            color: Some([0.2, 0.4, 0.5]),
            grain: None,
            leyes: vec![],
            builtin: None,
        });
        assert!(p2.fluir_params(hijo).is_some(), "el hijo hereda Fluir del padre");
    }

    #[test]
    fn herencia_con_ciclo_no_cuelga() {
        let mut p = Project::starter();
        let a = p.alloc_id();
        let b = p.alloc_id();
        p.materiales.push(MaterialDef { id: a, name: "a".into(), parent: Some(b), role: MatRole::Terreno, color: None, grain: None, leyes: vec![], builtin: None });
        p.materiales.push(MaterialDef { id: b, name: "b".into(), parent: Some(a), role: MatRole::Terreno, color: None, grain: None, leyes: vec![], builtin: None });
        // No debe colgarse; cae al gris neutro.
        let r = p.resolve_material(a);
        assert_eq!(r.color, [150, 150, 150]);
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
    fn ventana_de_escena_sigue_al_reparto() {
        let dim = world_dim(128); // [128, 51, 128] → medio = 64
        // Un actor que camina lejos del origen (más allá de la caja finita vieja):
        // un keyframe en grilla 18, otro en 600 → coords de MUNDO, no acotadas.
        let s = SceneSpec {
            id: 0,
            name: "caminata larga".into(),
            mundo: 0,
            duration: 10.0,
            actors: vec![ActorSpec {
                character: 0,
                keys: vec![
                    ActorKeySpec { t: 0.0, gx: 18.0, gz: 64.0, clip: None, face: None },
                    ActorKeySpec { t: 10.0, gx: 600.0, gz: 64.0, clip: None, face: None },
                ],
                frame_rate: None,
            }],
            shots: vec![],
            handheld: 0.0,
        };
        let scripts = s.scripts();

        // Al final el reparto está cerca de wx=600: la ventana lo centra (su columna
        // local cae cerca de dim/2), cosa imposible en una caja fija centrada en 0.
        let o_fin = window_origin_for_cast(&scripts, 10.0, dim);
        let local_x = 600 - o_fin[0];
        assert!(
            (local_x - dim[0] as i32 / 2).abs() <= SCENE_WINDOW_STEP,
            "el reparto queda centrado en la ventana (local_x={local_x})"
        );

        // El origen se snappea al paso (sólo salta a múltiplos) → no regenera por cuadro.
        assert_eq!(o_fin[0] % SCENE_WINDOW_STEP, 0);
        assert_eq!(o_fin[1] % SCENE_WINDOW_STEP, 0);

        // Y la ventana se MUEVE entre el principio y el final (mundo infinito, no caja).
        let o_ini = window_origin_for_cast(&scripts, 0.0, dim);
        assert_ne!(o_ini[0], o_fin[0], "la ventana siguió al reparto en X");

        // Sin reparto: mundo centrado en el origen (compatibilidad con el preview fijo).
        assert_eq!(window_origin_for_cast(&[], 0.0, dim), [0, 0]);
    }

    #[test]
    fn starter_trae_escena_y_personajes() {
        let p = Project::starter();
        assert_eq!(p.seres.len(), 3);
        assert_eq!(p.escenas.len(), 1);
        // La escena referencia un mundo válido por id.
        assert!(p.mundo(p.escenas[0].mundo).is_some());
        // Los 6 materiales semilla están sembrados.
        assert_eq!(p.materiales.len(), 6);
        assert!(p.material_id_for(Material::Sand).is_some());
    }
}
