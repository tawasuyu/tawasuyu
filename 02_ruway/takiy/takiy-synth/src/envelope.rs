//! `Adsr` — envolvente Attack/Decay/Sustain/Release medida en segundos.
//!
//! Modela el contorno de amplitud típico de un sintetizador. La nota
//! tiene una duración fija (`note_duration`): mientras el tiempo está
//! dentro de ella sigue attack/decay/sustain; cuando termina entra en
//! release y se apaga.

/// Envolvente ADSR. Tiempos en segundos; `sustain` es nivel en `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Adsr {
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

impl Adsr {
    /// Default razonable para el MVP: 10ms attack, 50ms decay, 70%
    /// sustain, 100ms release. Suena "a teclado de juguete".
    pub const DEFAULT: Adsr = Adsr {
        attack: 0.01,
        decay: 0.05,
        sustain: 0.7,
        release: 0.1,
    };

    /// Nivel de amplitud `[0, 1]` en el instante `t` (segundos desde el
    /// inicio de la nota), siendo `note_duration` la duración antes del
    /// release. Devuelve `0.0` si `t` está antes de la nota o después
    /// del final del release.
    pub fn level(&self, t: f32, note_duration: f32) -> f32 {
        if t < 0.0 || t >= note_duration + self.release {
            return 0.0;
        }

        if t < note_duration {
            self.level_on(t)
        } else {
            // Release desde el nivel que tenía justo al apagar la nota.
            let on_level = self.level_on(note_duration);
            let r = (t - note_duration) / self.release.max(1e-9);
            on_level * (1.0 - r).max(0.0)
        }
    }

    /// Nivel mientras la nota está sonando (sin contar release).
    fn level_on(&self, t: f32) -> f32 {
        let a = self.attack.max(1e-9);
        let d = self.decay.max(1e-9);
        if t < self.attack {
            t / a
        } else if t < self.attack + self.decay {
            let frac = (t - self.attack) / d;
            1.0 - frac * (1.0 - self.sustain)
        } else {
            self.sustain
        }
    }
}

impl Default for Adsr {
    fn default() -> Self {
        Adsr::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn before_note_is_silent() {
        assert_eq!(Adsr::DEFAULT.level(-0.5, 1.0), 0.0);
    }

    #[test]
    fn after_release_is_silent() {
        let env = Adsr::DEFAULT;
        assert_eq!(env.level(1.0 + env.release + 0.01, 1.0), 0.0);
    }

    #[test]
    fn attack_ramps_from_zero_to_one() {
        let env = Adsr { attack: 0.1, decay: 0.1, sustain: 0.5, release: 0.1 };
        assert!(env.level(0.0, 1.0).abs() < 1e-6);
        assert!((env.level(0.05, 1.0) - 0.5).abs() < 1e-6);
        assert!((env.level(0.1, 1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn sustains_at_sustain_level() {
        let env = Adsr { attack: 0.01, decay: 0.01, sustain: 0.5, release: 0.1 };
        assert!((env.level(0.5, 1.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn release_decays_from_sustain_to_zero() {
        let env = Adsr { attack: 0.01, decay: 0.01, sustain: 0.5, release: 0.2 };
        // Justo después del note_off el nivel es ≈sustain.
        assert!((env.level(1.0 + 1e-5, 1.0) - 0.5).abs() < 1e-3);
        // A mitad del release está a la mitad de sustain.
        assert!((env.level(1.0 + 0.1, 1.0) - 0.25).abs() < 1e-3);
        // Al final del release, silencio.
        assert!(env.level(1.0 + 0.2, 1.0).abs() < 1e-6);
    }

    #[test]
    fn short_note_releases_from_attack_level_not_sustain() {
        // Nota más corta que el attack: nunca llega al pico, así que el
        // release debe partir del nivel parcial.
        let env = Adsr { attack: 1.0, decay: 0.1, sustain: 0.5, release: 0.1 };
        // Nota dura 0.5s → al apagar está a la mitad del attack = 0.5.
        let lvl_at_off = env.level(0.5 + 1e-5, 0.5);
        assert!((lvl_at_off - 0.5).abs() < 1e-3);
    }
}
