//! `director` — una **timeline guionada** para filmar: describe qué hace cada
//! actor y qué hace la cámara *en función del tiempo*, de forma **determinista**
//! (mismo `t` → mismo estado → reproducible cuadro a cuadro). Es la capa de
//! *dirección* que faltaba: en vez de hardcodear el bucle de la escena, se
//! declara un [`Sequence`] (el "guion") y se reproduce.
//!
//! Modelo, deliberadamente chico:
//! - [`ActorScript`] = keyframes `(t, posición de grilla, clip?, rumbo?)` de un
//!   actor; `sample(t)` interpola la posición y decide el [`Clip`] (auto: camina
//!   si se mueve, quieto si no) y el rumbo (auto: dirección de marcha).
//! - [`Shot`] = un plano de cámara ([`CameraTrack`]) con su instante de inicio;
//!   varios planos dan **cortes duros** (cambia de plano sin interpolar).
//! - [`Sequence`] = el reparto de scripts + la lista de planos + duración.
//!
//! Es **contenido puro** (coordenadas de grilla, sin terreno ni GPU): el que
//! reproduce (la app) mapea la posición de grilla al relieve y posa cada
//! [`Actor`](crate::Actor) (con su cross-fade de clips). Reusable por cualquier
//! película/juego voxel.

use llimphi_3d::glam::Vec3;
use llimphi_3d::{Camera3d, CameraTrack};

use crate::actor::Clip;

/// Umbral de desplazamiento (unidades de grilla por segmento) para considerar
/// que un actor "se mueve" (y por tanto camina y mira hacia donde va).
const MOVING_EPS: f32 = 0.05;

/// Envuelve un ángulo a `[-π, π]` (la diferencia más corta entre dos rumbos).
fn wrap_pi(mut a: f32) -> f32 {
    use std::f32::consts::PI;
    while a > PI {
        a -= 2.0 * PI;
    }
    while a < -PI {
        a += 2.0 * PI;
    }
    a
}

/// Un keyframe de actor: dónde está (grilla), opcionalmente qué clip reproduce y
/// hacia dónde mira. `clip`/`face` en `None` = automático.
#[derive(Debug, Clone, Copy)]
pub struct ActorKey {
    /// Instante (segundos).
    pub t: f32,
    /// Posición de grilla en `t`.
    pub gx: f32,
    pub gz: f32,
    /// Clip explícito mientras se sale de esta key (`None` = auto: caminar/quieto).
    pub clip: Option<Clip>,
    /// Rumbo explícito (yaw; `None` = auto: dirección de marcha).
    pub face: Option<f32>,
}

impl ActorKey {
    /// Key en `(gx, gz)` a tiempo `t`, clip/rumbo automáticos.
    pub fn at(t: f32, gx: f32, gz: f32) -> Self {
        Self { t, gx, gz, clip: None, face: None }
    }

    /// Fija el clip a reproducir desde esta key (p.ej. quedarse saludando).
    pub fn play(mut self, clip: Clip) -> Self {
        self.clip = Some(clip);
        self
    }

    /// Fija el rumbo (yaw) desde esta key (p.ej. mirar a la cámara al detenerse).
    pub fn facing(mut self, yaw: f32) -> Self {
        self.face = Some(yaw);
        self
    }
}

/// Lo que un [`ActorScript`] dicta para un instante: dónde poner al actor
/// (grilla), hacia dónde mirar y qué clip animar.
#[derive(Debug, Clone, Copy)]
pub struct ActorSample {
    pub gx: f32,
    pub gz: f32,
    pub facing: f32,
    pub clip: Clip,
}

/// El guion de **un** actor: una lista de [`ActorKey`] ordenada en el tiempo.
#[derive(Debug, Clone, Default)]
pub struct ActorScript {
    keys: Vec<ActorKey>,
}

impl ActorScript {
    /// Crea el guion (ordena las keys por `t`).
    pub fn new(mut keys: Vec<ActorKey>) -> Self {
        keys.sort_by(|a, b| a.t.total_cmp(&b.t));
        Self { keys }
    }

    /// Tiempo de la última key (duración del guion del actor).
    pub fn duration(&self) -> f32 {
        self.keys.last().map(|k| k.t).unwrap_or(0.0)
    }

    /// Instantes (segundos) en que este actor **arranca un gesto** (clip emote
    /// explícito en una key). Son momentos expresivos del guion — el director los usa
    /// para acentuar la música.
    pub fn emote_onsets(&self) -> Vec<f32> {
        self.keys.iter().filter(|k| k.clip.is_some_and(|c| c.is_emote())).map(|k| k.t).collect()
    }

    /// El estado dictado en `t`: posición interpolada + clip + rumbo. Antes/
    /// después del rango, clampa a la primera/última key (quieto).
    pub fn sample(&self, t: f32) -> ActorSample {
        let keys = &self.keys;
        if keys.is_empty() {
            return ActorSample { gx: 0.0, gz: 0.0, facing: 0.0, clip: Clip::Idle };
        }
        let last = keys.len() - 1;
        // Extremos: quieto en la primera/última key.
        if t <= keys[0].t || last == 0 {
            let k = &keys[0];
            return ActorSample {
                gx: k.gx,
                gz: k.gz,
                facing: k.face.unwrap_or(0.0),
                clip: k.clip.unwrap_or(Clip::Idle),
            };
        }
        if t >= keys[last].t {
            let k = &keys[last];
            return ActorSample {
                gx: k.gx,
                gz: k.gz,
                facing: k.face.unwrap_or_else(|| self.motion_facing(last.saturating_sub(1))),
                clip: k.clip.unwrap_or(Clip::Idle),
            };
        }
        // Segmento `[i, i+1]` que contiene a `t`.
        let i = keys.iter().rposition(|k| k.t <= t).unwrap_or(0).min(last - 1);
        let (a, b) = (&keys[i], &keys[i + 1]);
        let f = ((t - a.t) / (b.t - a.t).max(1e-6)).clamp(0.0, 1.0);
        let (dx, dz) = (b.gx - a.gx, b.gz - a.gz);
        let moving = dx.hypot(dz) > MOVING_EPS;
        // Rumbo: si el guion lo fija en ambas keys, **gira suave** entre ambas
        // (por el camino más corto); si sólo en la de salida, lo mantiene; si en
        // ninguna, automático (dirección de marcha, o la última al detenerse).
        let facing = match (a.face, b.face) {
            (Some(fa), Some(fb)) => fa + wrap_pi(fb - fa) * f,
            (Some(fa), None) => fa,
            (None, _) if moving => dx.atan2(dz), // yaw=0 → +Z, como Actor::face_towards
            (None, _) => self.motion_facing(i),
        };
        ActorSample {
            gx: a.gx + dx * f,
            gz: a.gz + dz * f,
            clip: a.clip.unwrap_or(if moving { Clip::Walk } else { Clip::Idle }),
            facing,
        }
    }

    /// Rumbo del último segmento *con movimiento* hasta el índice `i` (para
    /// mantener la orientación cuando el actor está detenido); `0` si nunca se
    /// movió.
    fn motion_facing(&self, i: usize) -> f32 {
        for j in (0..=i).rev() {
            if j + 1 < self.keys.len() {
                let (a, b) = (&self.keys[j], &self.keys[j + 1]);
                let (dx, dz) = (b.gx - a.gx, b.gz - a.gz);
                if dx.hypot(dz) > MOVING_EPS {
                    return dx.atan2(dz);
                }
            }
        }
        0.0
    }
}

/// Un **plano** de cámara: una [`CameraTrack`] que arranca en `start` (segundos
/// absolutos de la secuencia). Sus keys son **relativas** al plano (`t=0` =
/// inicio del plano), así un plano es autocontenido y reubicable.
#[derive(Debug, Clone)]
pub struct Shot {
    pub start: f32,
    pub track: CameraTrack,
}

impl Shot {
    pub fn new(start: f32, track: CameraTrack) -> Self {
        Self { start, track }
    }
}

/// El **guion completo**: el reparto (un [`ActorScript`] por actor), los planos
/// de cámara (con cortes duros entre ellos) y la duración total. Reproducir =
/// para cada `t`: `camera(t)` + `actor.sample(t)` por actor.
#[derive(Debug, Clone, Default)]
pub struct Sequence {
    pub actors: Vec<ActorScript>,
    shots: Vec<Shot>,
    pub duration: f32,
}

impl Sequence {
    /// Crea la secuencia (ordena los planos por inicio).
    pub fn new(actors: Vec<ActorScript>, mut shots: Vec<Shot>, duration: f32) -> Self {
        shots.sort_by(|a, b| a.start.total_cmp(&b.start));
        Self { actors, shots, duration }
    }

    /// Cantidad de cuadros a `fps` para la duración total.
    pub fn frames(&self, fps: u32) -> u32 {
        (self.duration * fps as f32).round() as u32
    }

    /// La cámara en `t`: el plano activo (el último cuyo `start ≤ t`) muestreado a
    /// `t − start`. El salto entre planos es un **corte duro** (sin interpolar).
    pub fn camera(&self, t: f32) -> Camera3d {
        let shot = self
            .shots
            .iter()
            .rev()
            .find(|s| s.start <= t)
            .or_else(|| self.shots.first());
        match shot {
            Some(s) => s.track.sample(t - s.start),
            None => Camera3d::default(),
        }
    }

    /// Los **"beats del guion"**: los instantes (segundos, ordenados, sin repetir)
    /// que merecen un acento musical — los **cortes de cámara** (inicio de cada plano
    /// salvo el primero) y los **gestos** de los actores. Es lo que deja que la banda
    /// sonora caiga *sobre la acción* en vez de sólo compartir duración. Dos tiempos a
    /// menos de `EPS` se consideran el mismo acento.
    pub fn beat_times(&self) -> Vec<f32> {
        const EPS: f32 = 0.05;
        let mut ts: Vec<f32> = Vec::new();
        // Cortes de cámara (el primer plano no es un corte).
        for s in self.shots.iter().skip(1) {
            ts.push(s.start);
        }
        // Gestos de cada actor.
        for a in &self.actors {
            ts.extend(a.emote_onsets());
        }
        ts.retain(|&t| t >= 0.0 && t <= self.duration + EPS);
        ts.sort_by(f32::total_cmp);
        ts.dedup_by(|a, b| (*a - *b).abs() < EPS);
        ts
    }

    /// Posición de grilla (sin altura) del **centroide** del reparto en `t`,
    /// útil para apuntar la cámara al grupo. `None` si no hay actores.
    pub fn cast_centroid(&self, t: f32) -> Option<Vec3> {
        if self.actors.is_empty() {
            return None;
        }
        let mut sx = 0.0;
        let mut sz = 0.0;
        for a in &self.actors {
            let s = a.sample(t);
            sx += s.gx;
            sz += s.gz;
        }
        let n = self.actors.len() as f32;
        Some(Vec3::new(sx / n, 0.0, sz / n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    #[test]
    fn camina_y_luego_emota() {
        // De (0,0) a (10,0) en 2 s, después se queda saludando.
        let s = ActorScript::new(vec![
            ActorKey::at(0.0, 0.0, 0.0),
            ActorKey::at(2.0, 10.0, 0.0).play(Clip::Wave).facing(FRAC_PI_2),
        ]);
        // A mitad del trayecto: posición media, caminando, mirando a +X.
        let m = s.sample(1.0);
        assert!((m.gx - 5.0).abs() < 1e-4);
        assert_eq!(m.clip, Clip::Walk);
        assert!((m.facing - FRAC_PI_2).abs() < 1e-4, "mueve en +X → yaw=π/2");
        // Al final: quieto, saludando, mirando a donde dijimos.
        let e = s.sample(3.0);
        assert_eq!((e.gx, e.clip), (10.0, Clip::Wave));
        assert!((e.facing - FRAC_PI_2).abs() < 1e-4);
    }

    #[test]
    fn beats_del_guion_son_cortes_y_gestos() {
        use llimphi_3d::CamKey;
        // Actor que camina y a los 3 s saluda; otro que a los 4 s señala.
        let a1 = ActorScript::new(vec![
            ActorKey::at(0.0, 0.0, 0.0),
            ActorKey::at(3.0, 5.0, 0.0).play(Clip::Wave),
        ]);
        let a2 = ActorScript::new(vec![
            ActorKey::at(0.0, 2.0, 0.0),
            ActorKey::at(4.0, 6.0, 0.0).play(Clip::Point),
        ]);
        let s0 = CameraTrack::new(vec![CamKey::look(0.0, Vec3::ZERO, Vec3::Z, 50.0)]);
        let s1 = CameraTrack::new(vec![CamKey::look(0.0, Vec3::Y, Vec3::Z, 50.0)]);
        // Dos planos: el corte está en 2.5 s (el primero no cuenta).
        let seq = Sequence::new(vec![a1, a2], vec![Shot::new(0.0, s0), Shot::new(2.5, s1)], 5.0);
        let beats = seq.beat_times();
        // Esperado: corte 2.5, saludo 3.0, señal 4.0 (ordenados, sin el inicio).
        assert_eq!(beats.len(), 3);
        assert!((beats[0] - 2.5).abs() < 1e-4);
        assert!((beats[1] - 3.0).abs() < 1e-4);
        assert!((beats[2] - 4.0).abs() < 1e-4);
    }

    #[test]
    fn cortes_de_camara_son_duros() {
        use llimphi_3d::CamKey;
        let near = CameraTrack::new(vec![CamKey::look(0.0, Vec3::ZERO, Vec3::Z, 50.0)]);
        let far = CameraTrack::new(vec![CamKey::look(0.0, Vec3::new(100.0, 0.0, 0.0), Vec3::Z, 50.0)]);
        let seq = Sequence::new(vec![], vec![Shot::new(0.0, near), Shot::new(1.0, far)], 2.0);
        // Antes del corte: plano cercano (eye en origen).
        assert!(seq.camera(0.5).eye.x.abs() < 1e-3);
        // Después del corte (t≥1): plano lejano, sin interpolación intermedia.
        assert!((seq.camera(1.5).eye.x - 100.0).abs() < 1e-3);
    }
}
