//! `conducta` — **capa 3 del movimiento**: cómo un Ser *decide* moverse por el
//! mundo. Una [`Conducta`] son parámetros de *steering* (declarativos, como las Leyes
//! y los Andares); un [`Habitante`] es el agente que los ejecuta sobre la física de
//! cuerpo del [`Player`] (gravedad + colisión), combinando **deambular** (rumbo que
//! cambia con la inquietud), **gregarismo** (se acerca a su manada) y **miedo** (huye
//! de una amenaza). Determinista (LCG por agente; sin `rand`).
//!
//! Es CPU puro: el agente expone `pos`/`heading`/`fase`, y la app lo pinta con el
//! cuerpo del Ser (rig + andar) vía [`CharSpec::to_meta`](crate::CharSpec::to_meta).

use llimphi_3d::glam::Vec3;
use llimphi_3d::VoxelGrid;
use serde::{Deserialize, Serialize};

use crate::ecuacion::{Expr, Symbols};
use crate::{forward_h, Player};

const TAU: f32 = std::f32::consts::TAU;
/// Radio (voxels) en el que un habitante "ve" a su manada y a la amenaza.
const RADIO_VISION: f32 = 12.0;

/// **Percepts** que una fórmula de peso de impulso puede leer, en orden (los
/// `Field` de la tabla de símbolos). Son lo que el agente "siente" cada paso,
/// normalizado a `[0,1]`:
/// - `cercania_manada` — 1 = centro de la manada pegado, 0 = lejos/sin manada.
/// - `cercania_amenaza` — 1 = amenaza encima, 0 = lejos/sin amenaza.
/// - `apinamiento` — fracción del campo de visión ocupada por vecinos.
/// - `azar` — ruido determinista por paso (no consume el LCG del deambular).
/// - `en_suelo` — 1 si está apoyado, 0 en el aire.
pub const PERCEPTS: [&str; 5] =
    ["cercania_manada", "cercania_amenaza", "apinamiento", "azar", "en_suelo"];
/// Nombres de los **parámetros** de la conducta (los sliders), en orden. También son
/// visibles como variables en las fórmulas de peso (`Param` de la tabla de símbolos).
pub const PARAMS: [&str; 5] = ["velocidad", "inquietud", "salto", "gregario", "miedo"];

/// **Primitiva de steering**: una dirección base que el agente sabe generar cada
/// paso. El [`Impulso`] la pondera con una fórmula autorable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Primitiva {
    /// Rumbo de deambular (heading con ruido).
    Deambular,
    /// Hacia el centro de la manada visible.
    Cohesion,
    /// Lejos del centro de la manada (anti‑apiñamiento).
    Separacion,
    /// Lejos de la amenaza.
    Huir,
}

impl Primitiva {
    /// Todas las primitivas, para ciclar en la UI.
    pub const TODAS: [Primitiva; 4] =
        [Primitiva::Deambular, Primitiva::Cohesion, Primitiva::Separacion, Primitiva::Huir];

    pub fn label(self) -> &'static str {
        match self {
            Primitiva::Deambular => "deambular",
            Primitiva::Cohesion => "cohesión",
            Primitiva::Separacion => "separación",
            Primitiva::Huir => "huir",
        }
    }

    /// La siguiente primitiva (ciclo) — para el botón «cambiar» de la UI.
    pub fn next(self) -> Primitiva {
        let i = Primitiva::TODAS.iter().position(|&p| p == self).unwrap_or(0);
        Primitiva::TODAS[(i + 1) % Primitiva::TODAS.len()]
    }
}

/// Un **impulso de steering autorable**: una [`Primitiva`] (dirección base) pesada por
/// una **fórmula** sobre percepts + params. La suma de los impulsos da la dirección
/// deseada del agente. La fórmula es el texto canónico (se compila con
/// [`Conducta::symbols`], igual que las Leyes de rejilla).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Impulso {
    pub primitiva: Primitiva,
    /// Fórmula del peso (texto; ej. `"gregario * 1.5"` o `"miedo * 2.5 * cercania_amenaza"`).
    pub peso: String,
}

impl Impulso {
    pub fn new(primitiva: Primitiva, peso: impl Into<String>) -> Self {
        Self { primitiva, peso: peso.into() }
    }
}

/// Los **impulsos por defecto**: reproducen exactamente el steering cableado histórico
/// (deambular + cohesión·`gregario·1.5` + huida·`miedo·2.5·cercania_amenaza`). Un
/// [`Conducta`] con `impulsos` vacío usa estos, así los datos viejos siguen andando.
pub fn default_impulsos() -> Vec<Impulso> {
    vec![
        Impulso::new(Primitiva::Deambular, "1"),
        Impulso::new(Primitiva::Cohesion, "gregario * 1.5"),
        Impulso::new(Primitiva::Huir, "miedo * 2.5 * cercania_amenaza"),
    ]
}

/// **Conducta** de un Ser: parámetros de locomoción (los sliders) + **impulsos de
/// steering autorables**. Declarativa y editable; la ejecuta un [`Habitante`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conducta {
    /// Velocidad de marcha (voxels/seg).
    pub velocidad: f32,
    /// **Inquietud** `[0,1]`: cuán seguido y fuerte cambia de rumbo al deambular.
    pub inquietud: f32,
    /// Probabilidad `[0,1]` de pegar un salto al caminar.
    pub salto: f32,
    /// **Gregarismo** `[0,1]`: parámetro visible en las fórmulas (peso de cohesión por
    /// defecto). No se usa directo: entra por la fórmula del impulso.
    pub gregario: f32,
    /// **Miedo** `[0,1]`: parámetro visible en las fórmulas (peso de huida por defecto).
    pub miedo: f32,
    /// **Impulsos de steering** autorables. Vacío = [`default_impulsos`] (compat).
    #[serde(default)]
    pub impulsos: Vec<Impulso>,
}

impl Default for Conducta {
    fn default() -> Self {
        Self {
            velocidad: 3.0,
            inquietud: 0.4,
            salto: 0.08,
            gregario: 0.35,
            miedo: 0.6,
            impulsos: default_impulsos(),
        }
    }
}

impl Conducta {
    /// Parámetros editables: `(nombre, valor, min, max)` — para armar sliders.
    pub fn params(&self) -> [(&'static str, f32, f32, f32); 5] {
        [
            ("velocidad", self.velocidad, 0.0, 6.0),
            ("inquietud", self.inquietud, 0.0, 1.0),
            ("salto", self.salto, 0.0, 1.0),
            ("gregario", self.gregario, 0.0, 1.0),
            ("miedo", self.miedo, 0.0, 1.0),
        ]
    }
    /// Fija el parámetro `i` (en el orden de [`params`](Self::params)).
    pub fn set(&mut self, i: usize, v: f32) {
        match i {
            0 => self.velocidad = v.clamp(0.0, 6.0),
            1 => self.inquietud = v.clamp(0.0, 1.0),
            2 => self.salto = v.clamp(0.0, 1.0),
            3 => self.gregario = v.clamp(0.0, 1.0),
            4 => self.miedo = v.clamp(0.0, 1.0),
            _ => {}
        }
    }

    /// Los valores de los params en el orden de [`PARAMS`] (entorno para las fórmulas).
    fn param_env(&self) -> [f32; 5] {
        [self.velocidad, self.inquietud, self.salto, self.gregario, self.miedo]
    }

    /// Tabla de símbolos de las fórmulas de peso: percepts como `Field`, params como
    /// `Param`. La comparte el editor (barra de fórmula) y el compilador del agente.
    pub fn symbols() -> Symbols {
        Symbols {
            campos: PERCEPTS.iter().map(|s| s.to_string()).collect(),
            params: PARAMS.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Los impulsos efectivos: los propios, o [`default_impulsos`] si está vacío.
    pub fn impulsos_efectivos(&self) -> Vec<Impulso> {
        if self.impulsos.is_empty() {
            default_impulsos()
        } else {
            self.impulsos.clone()
        }
    }

    /// Compila los impulsos efectivos a `(primitiva, Expr)`. Una fórmula que no parsea
    /// cae a peso `0` (impulso inerte) — el editor muestra el error aparte.
    fn compilar(&self) -> Vec<(Primitiva, Expr)> {
        let sym = Conducta::symbols();
        self.impulsos_efectivos()
            .iter()
            .map(|i| (i.primitiva, Expr::parse(&i.peso, &sym).unwrap_or(Expr::Const(0.0))))
            .collect()
    }
}

/// Un **habitante**: un agente que ejecuta una [`Conducta`] sobre la física del
/// [`Player`]. La app lo pinta con el cuerpo del Ser que representa.
#[derive(Debug, Clone)]
pub struct Habitante {
    body: Player,
    conducta: Conducta,
    /// Impulsos compilados (fórmula de peso ya parseada) — se rearman al editar la
    /// conducta, no en el loop caliente.
    impulsos: Vec<(Primitiva, Expr)>,
    /// Rumbo actual (yaw, rad; `0` mira a `+Z`).
    heading: f32,
    /// Fase de animación (avanza al caminar) para el andar del cuerpo.
    pub fase: f32,
    /// Cuenta atrás hasta el próximo cambio de rumbo (deambular).
    timer: f32,
    /// Pide saltar el próximo paso.
    jump: bool,
    /// LCG (azar determinista por habitante).
    rng: u32,
}

impl Habitante {
    /// Habitante posado sobre la columna `(x, z)` del grid, con su conducta y semilla.
    pub fn spawn(grid: &VoxelGrid, x: u32, z: u32, conducta: Conducta, seed: u32) -> Self {
        let mut body = Player::spawn_on(grid, x, z);
        body.speed = conducta.velocidad;
        let impulsos = conducta.compilar();
        let mut h = Self {
            body,
            conducta,
            impulsos,
            heading: 0.0,
            fase: 0.0,
            timer: 0.0,
            jump: false,
            rng: seed | 1,
        };
        h.heading = h.next() * TAU;
        h
    }

    /// Posición (espacio de grilla, igual que el terreno).
    pub fn pos(&self) -> Vec3 {
        self.body.pos
    }
    /// Rumbo actual (para orientar el cuerpo).
    pub fn heading(&self) -> f32 {
        self.heading
    }

    /// Actualiza la conducta en caliente (para reflejar ediciones sin re-spawnear).
    /// Recompila los impulsos (parseo de las fórmulas de peso).
    pub fn set_conducta(&mut self, c: Conducta) {
        self.impulsos = c.compilar();
        self.conducta = c;
    }

    /// Avanza `dt`: arma la **dirección deseada** como suma ponderada de los impulsos
    /// autorables (cada primitiva genera una dirección base; su fórmula de peso la
    /// escala según los percepts y params), y mueve el cuerpo. `vecinos` son las
    /// posiciones de los demás habitantes (la propia se ignora por distancia ~0).
    pub fn step(&mut self, grid: &VoxelGrid, vecinos: &[Vec3], amenaza: Option<Vec3>, dt: f32) {
        let velocidad = self.conducta.velocidad;
        let inquietud = self.conducta.inquietud;
        let salto = self.conducta.salto;

        // Deambular: cada tanto, empujar el rumbo (más inquietud = más seguido/fuerte).
        self.timer -= dt;
        if self.timer <= 0.0 {
            self.heading += (self.next() - 0.5) * (0.5 + 2.0 * inquietud);
            self.timer = (1.4 - inquietud).max(0.25) + self.next() * 1.5;
            self.jump = self.next() < salto;
        }

        let pos = self.body.pos;

        // --- Percepts (deterministas; NO consumen el LCG del deambular) ------------
        // Centro y densidad de la manada visible.
        let mut centro = Vec3::ZERO;
        let mut n = 0.0f32;
        for &v in vecinos {
            let d = (v - pos).length();
            if d > 0.3 && d < RADIO_VISION {
                centro += v;
                n += 1.0;
            }
        }
        let (hacia_manada, cercania_manada) = if n > 0.0 {
            let rel = (centro / n) - pos;
            let d = rel.length();
            (horiz_norm(rel), (1.0 - d / RADIO_VISION).clamp(0.0, 1.0))
        } else {
            (Vec3::ZERO, 0.0)
        };
        let apinamiento = (n / 8.0).min(1.0);

        let (lejos_amenaza, cercania_amenaza) = match amenaza {
            Some(a) => {
                let d = (pos - a).length();
                if d > 0.01 && d < RADIO_VISION {
                    (horiz_norm(pos - a), 1.0 - d / RADIO_VISION)
                } else {
                    (Vec3::ZERO, 0.0)
                }
            }
            None => (Vec3::ZERO, 0.0),
        };

        // Ruido determinista por paso, sin avanzar el LCG (mezcla del estado actual):
        // así el orden de sorteos del deambular no cambia y la sim sigue reproducible.
        let azar = {
            let mut h = self.rng;
            h ^= h >> 13;
            h = h.wrapping_mul(0x9E37_79B1);
            h ^= h >> 16;
            (h >> 8) as f32 / (1u32 << 24) as f32
        };
        let en_suelo = if self.body.on_ground { 1.0 } else { 0.0 };

        let percepts = [cercania_manada, cercania_amenaza, apinamiento, azar, en_suelo];
        let params = self.conducta.param_env();

        // --- Suma ponderada de los impulsos autorables ----------------------------
        let mut desired = Vec3::ZERO;
        for (prim, peso) in &self.impulsos {
            let dir = match prim {
                Primitiva::Deambular => forward_h(self.heading),
                Primitiva::Cohesion => hacia_manada,
                Primitiva::Separacion => -hacia_manada,
                Primitiva::Huir => lejos_amenaza,
            };
            let w = peso.eval_scalar(&percepts, &params, dt);
            desired += dir * w;
        }

        // Girar el rumbo hacia la dirección deseada (suave, con tope por paso).
        let desired = horiz_norm(desired);
        if desired.length_squared() > 1e-4 {
            let objetivo = desired.x.atan2(desired.z);
            let giro_max = (4.0 + 6.0 * inquietud) * dt;
            self.heading = acercar_angulo(self.heading, objetivo, giro_max);
        }

        // Mover el cuerpo; rebotar si choca (no quedar empujando una pared).
        self.body.speed = velocidad;
        let before = self.body.pos;
        let jump = self.jump && self.body.on_ground;
        self.body.step(grid, forward_h(self.heading), jump, dt);
        self.jump = false;

        let movido = (self.body.pos - before).length();
        if self.body.on_ground && movido < 0.2 * velocidad * dt {
            self.heading += TAU * 0.5 + (self.next() - 0.5);
            self.timer = 0.5;
        }
        // La fase de animación avanza con el movimiento (para el andar de caminata).
        self.fase += dt * (3.0 + velocidad);
    }

    /// Próximo `f32` en `[0,1)` del LCG.
    fn next(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// Normaliza un vector en el plano horizontal (Y=0); cero si es ~nulo.
fn horiz_norm(v: Vec3) -> Vec3 {
    let h = Vec3::new(v.x, 0.0, v.z);
    let l = h.length();
    if l > 1e-5 {
        h / l
    } else {
        Vec3::ZERO
    }
}

/// Acerca `a` hacia `b` (ángulos, rad) a lo sumo `max` por paso, por el lado corto.
fn acercar_angulo(a: f32, b: f32, max: f32) -> f32 {
    let mut d = (b - a) % TAU;
    if d > std::f32::consts::PI {
        d -= TAU;
    }
    if d < -std::f32::consts::PI {
        d += TAU;
    }
    a + d.clamp(-max, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Arena 48×48 con piso y **paredes** perimetrales (corral): los habitantes
    /// rebotan y no se caen por el borde.
    fn grid_con_piso() -> VoxelGrid {
        let n = 48u32;
        let mut g = VoxelGrid::new([n, 8, n]);
        for z in 0..n {
            for x in 0..n {
                g.set(x, 0, z, [90, 140, 80]);
                if x == 0 || x == n - 1 || z == 0 || z == n - 1 {
                    for y in 1..6 {
                        g.set(x, y, z, [120, 120, 120]);
                    }
                }
            }
        }
        g
    }

    #[test]
    fn deambula_y_no_atraviesa_el_piso() {
        let g = grid_con_piso();
        let mut h = Habitante::spawn(&g, 24, 24, Conducta::default(), 7);
        let start = h.pos();
        for _ in 0..600 {
            h.step(&g, &[], None, 1.0 / 60.0);
            assert!(h.pos().y >= 1.0 - 0.05, "se hundió: y={}", h.pos().y);
        }
        assert!((h.pos() - start).length() > 1.0, "no deambuló");
    }

    #[test]
    fn gregario_los_junta_mas_que_indiferentes() {
        let g = grid_con_piso();
        // Dos pares lejos entre sí; uno gregario, otro no. Tras un rato, los gregarios
        // quedan más cerca entre sí que los indiferentes.
        let separacion = |gregario: f32, seed: u32| {
            let mut c = Conducta::default();
            c.gregario = gregario;
            c.inquietud = 0.2;
            let mut a = Habitante::spawn(&g, 18, 24, c.clone(), seed);
            let mut b = Habitante::spawn(&g, 30, 24, c, seed + 100);
            for _ in 0..900 {
                let pa = a.pos();
                let pb = b.pos();
                a.step(&g, &[pb], None, 1.0 / 60.0);
                b.step(&g, &[pa], None, 1.0 / 60.0);
            }
            (a.pos() - b.pos()).length()
        };
        let juntos = separacion(1.0, 1);
        let sueltos = separacion(0.0, 1);
        assert!(juntos < sueltos, "gregarios más cerca ({juntos:.1}) que sueltos ({sueltos:.1})");
    }

    #[test]
    fn impulsos_por_defecto_parsean_y_reproducen_los_pesos() {
        // Las fórmulas de fábrica compilan contra los símbolos de la conducta, y sus
        // pesos evalúan a los valores cableados históricos.
        let sym = Conducta::symbols();
        let c = Conducta::default();
        let env_params = c.param_env();
        // percepts: cercania_amenaza=1 (amenaza encima).
        let percepts = [0.0, 1.0, 0.0, 0.0, 1.0];
        for imp in default_impulsos() {
            let e = Expr::parse(&imp.peso, &sym).expect("la fórmula de fábrica parsea");
            let w = e.eval_scalar(&percepts, &env_params, 1.0 / 60.0);
            match imp.primitiva {
                Primitiva::Deambular => assert!((w - 1.0).abs() < 1e-4),
                // gregario=0.35 → 0.35*1.5 = 0.525
                Primitiva::Cohesion => assert!((w - 0.525).abs() < 1e-4, "cohesión peso={w}"),
                // miedo=0.6, cercania=1 → 0.6*2.5*1 = 1.5
                Primitiva::Huir => assert!((w - 1.5).abs() < 1e-4, "huida peso={w}"),
                _ => {}
            }
        }
    }

    #[test]
    fn impulso_autorado_cambia_la_conducta() {
        // Un ser con SÓLO cohesión fuerte se junta más que uno con SÓLO separación,
        // partiendo de la misma posición y semilla: la fórmula autorada manda.
        let g = grid_con_piso();
        let corrida = |imp: Impulso| {
            let mut c = Conducta::default();
            c.inquietud = 0.1;
            c.impulsos = vec![Impulso::new(Primitiva::Deambular, "0.2"), imp];
            let mut a = Habitante::spawn(&g, 18, 24, c.clone(), 1);
            let mut b = Habitante::spawn(&g, 30, 24, c, 101);
            for _ in 0..900 {
                let pa = a.pos();
                let pb = b.pos();
                a.step(&g, &[pb], None, 1.0 / 60.0);
                b.step(&g, &[pa], None, 1.0 / 60.0);
            }
            (a.pos() - b.pos()).length()
        };
        let junta = corrida(Impulso::new(Primitiva::Cohesion, "2"));
        let separa = corrida(Impulso::new(Primitiva::Separacion, "2"));
        assert!(junta < separa, "cohesión junta ({junta:.1}) vs separación aleja ({separa:.1})");
    }

    #[test]
    fn determinismo_con_impulsos_autorados() {
        // Dos corridas idénticas de un ser con impulsos autorados dan el mismo recorrido:
        // el percept `azar` no rompe el determinismo (no consume el LCG).
        let g = grid_con_piso();
        let run = || {
            let mut c = Conducta::default();
            c.impulsos = vec![
                Impulso::new(Primitiva::Deambular, "1 + azar"),
                Impulso::new(Primitiva::Huir, "miedo * cercania_amenaza"),
            ];
            let amenaza = Some(Vec3::new(24.5, 1.0, 24.5));
            let mut h = Habitante::spawn(&g, 20, 24, c, 9);
            for _ in 0..500 {
                h.step(&g, &[], amenaza, 1.0 / 60.0);
            }
            h.pos()
        };
        assert_eq!(run(), run(), "misma evolución bit a bit");
    }

    #[test]
    fn miedo_aleja_de_la_amenaza() {
        let g = grid_con_piso();
        let amenaza = Vec3::new(24.5, 1.0, 24.5);
        let mut c = Conducta::default();
        c.miedo = 1.0;
        c.inquietud = 0.1;
        let mut h = Habitante::spawn(&g, 22, 24, c, 5); // arranca cerca de la amenaza
        let d0 = (h.pos() - amenaza).length();
        for _ in 0..600 {
            h.step(&g, &[], Some(amenaza), 1.0 / 60.0);
        }
        let d1 = (h.pos() - amenaza).length();
        assert!(d1 > d0 + 2.0, "huyó de la amenaza: {d0:.1} → {d1:.1}");
    }
}
