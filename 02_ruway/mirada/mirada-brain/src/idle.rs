//! `idle` — la política de **inactividad**: cuándo apagar la pantalla y cuándo
//! bloquear la sesión, con **consciencia de multimedia**.
//!
//! Es una state-machine pura y testeable (como el resto del Cerebro): no toca
//! `smithay` ni el hardware. El Cuerpo le inyecta el paso del tiempo
//! ([`tick`](IdleManager::tick)) y los hechos de entrada ([`activity`](IdleManager::activity)),
//! y ella devuelve [`IdleAction`]s que el Cuerpo ejecuta (DPMS, lock).
//!
//! **Consciencia de multimedia.** Mientras un cliente mantenga un *idle
//! inhibitor* (`zwp_idle_inhibit`, lo ponen los reproductores de vídeo/llamadas),
//! `tick(_, inhibited = true)` **no acumula** inactividad: la pantalla no se apaga
//! ni se bloquea aunque no toques el teclado, porque estás mirando algo. Al soltar
//! el inhibidor (terminó el vídeo) el contador parte de cero otra vez.
//!
//! **Dos umbrales independientes** (`0 = desactivado`): `screen_off_secs` (apagar
//! la pantalla, DPMS) y `lock_secs` (bloquear). Suelen combinarse —apagar y
//! bloquear— pero cada uno tiene su tiempo: apagar la pantalla a los 5 min,
//! bloquear a los 10, por ejemplo.

/// Config de la política de inactividad. Vive en la [`Config`](crate::Config) del
/// WM (editable desde wawa-panel) y se empuja con [`IdleManager::set_config`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdleConfig {
    /// Segundos de inactividad antes de **apagar la pantalla** (DPMS off).
    /// `0` = nunca.
    pub screen_off_secs: u32,
    /// Segundos de inactividad antes de **bloquear la sesión** (el lock del
    /// greeter). `0` = nunca.
    pub lock_secs: u32,
    /// Si `true`, un *idle inhibitor* de un cliente (vídeo en reproducción)
    /// **pausa** los contadores: ni se apaga ni se bloquea mientras mirás algo.
    pub respect_inhibitors: bool,
}

impl Default for IdleConfig {
    fn default() -> Self {
        // Por defecto **desactivado** (ambos umbrales en 0): el auto-lock y el
        // apagado por inactividad son opt-in desde la config, no sorpresas.
        IdleConfig {
            screen_off_secs: 0,
            lock_secs: 0,
            respect_inhibitors: true,
        }
    }
}

/// Lo que el Cuerpo debe ejecutar al cruzar un umbral o al despertar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleAction {
    /// Apagar la pantalla (DPMS off en DRM).
    ScreenOff,
    /// Encender la pantalla (DPMS on) — la actividad despertó al usuario.
    ScreenOn,
    /// Bloquear la sesión (componer el shell de credenciales).
    Lock,
}

/// La máquina de inactividad: acumula tiempo ocioso y dispara las acciones al
/// cruzar cada umbral. Edge-triggered: cada umbral dispara **una** vez por
/// período de inactividad; la actividad rearma.
#[derive(Debug, Clone)]
pub struct IdleManager {
    cfg: IdleConfig,
    /// Milisegundos acumulados sin actividad (ni input ni —si se respetan—
    /// inhibidores).
    idle_ms: u64,
    /// Ya se disparó el apagado de pantalla en este período (no re-disparar).
    screen_off: bool,
    /// Ya se disparó el bloqueo en este período.
    locked: bool,
}

impl IdleManager {
    /// Nueva máquina con la config dada (típicamente [`IdleConfig::default`] =
    /// desactivada hasta que la config del usuario diga otra cosa).
    pub fn new(cfg: IdleConfig) -> Self {
        IdleManager {
            cfg,
            idle_ms: 0,
            screen_off: false,
            locked: false,
        }
    }

    /// Reemplaza la config (recarga en caliente desde wawa-panel). No toca el
    /// contador en curso; los umbrales nuevos rigen desde el próximo `tick`.
    pub fn set_config(&mut self, cfg: IdleConfig) {
        self.cfg = cfg;
    }

    /// ¿Está la pantalla apagada según la política? (para que el Cuerpo no
    /// re-emita DPMS).
    pub fn is_screen_off(&self) -> bool {
        self.screen_off
    }

    /// Hubo **actividad** (input del usuario): reinicia el contador y, si la
    /// pantalla estaba apagada por inactividad, pide encenderla. El lock **no**
    /// se deshace acá (lo resuelve el desbloqueo); sólo se rearma el disparo.
    pub fn activity(&mut self) -> Vec<IdleAction> {
        self.idle_ms = 0;
        self.locked = false;
        let mut actions = Vec::new();
        if self.screen_off {
            self.screen_off = false;
            actions.push(IdleAction::ScreenOn);
        }
        actions
    }

    /// Avanza `dt_ms` con el estado de inhibición actual (`inhibited` = hay un
    /// cliente con idle-inhibitor, p. ej. vídeo). Devuelve las acciones que
    /// cruzaron su umbral en este paso.
    pub fn tick(&mut self, dt_ms: u64, inhibited: bool) -> Vec<IdleAction> {
        // Consciencia de multimedia: un inhibidor activo cuenta como actividad
        // (pausa y rearma), así nada se apaga ni bloquea mientras mirás vídeo.
        if self.cfg.respect_inhibitors && inhibited {
            return self.activity();
        }
        self.idle_ms = self.idle_ms.saturating_add(dt_ms);
        let mut actions = Vec::new();
        // Apagado de pantalla.
        if self.cfg.screen_off_secs > 0
            && !self.screen_off
            && self.idle_ms >= self.cfg.screen_off_secs as u64 * 1000
        {
            self.screen_off = true;
            actions.push(IdleAction::ScreenOff);
        }
        // Bloqueo.
        if self.cfg.lock_secs > 0
            && !self.locked
            && self.idle_ms >= self.cfg.lock_secs as u64 * 1000
        {
            self.locked = true;
            actions.push(IdleAction::Lock);
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(off: u32, lock: u32) -> IdleConfig {
        IdleConfig {
            screen_off_secs: off,
            lock_secs: lock,
            respect_inhibitors: true,
        }
    }

    #[test]
    fn desactivado_por_defecto_no_dispara_nada() {
        let mut m = IdleManager::new(IdleConfig::default());
        // Una hora ociosa, sin umbrales: nada.
        assert!(m.tick(3_600_000, false).is_empty());
        assert!(!m.is_screen_off());
    }

    #[test]
    fn apaga_pantalla_y_bloquea_en_su_umbral_una_sola_vez() {
        let mut m = IdleManager::new(cfg(5, 10)); // 5 s apagar, 10 s bloquear
        assert!(m.tick(4_000, false).is_empty());
        // Cruza los 5 s: apaga.
        assert_eq!(m.tick(2_000, false), vec![IdleAction::ScreenOff]);
        assert!(m.is_screen_off());
        // Sigue ocioso pero antes de 10 s: nada nuevo.
        assert!(m.tick(2_000, false).is_empty());
        // Cruza los 10 s: bloquea.
        assert_eq!(m.tick(2_000, false), vec![IdleAction::Lock]);
        // Más tiempo: ningún re-disparo.
        assert!(m.tick(10_000, false).is_empty());
    }

    #[test]
    fn actividad_enciende_la_pantalla_y_rearma() {
        let mut m = IdleManager::new(cfg(5, 0));
        m.tick(6_000, false); // apaga
        assert!(m.is_screen_off());
        // El usuario vuelve: enciende y rearma.
        assert_eq!(m.activity(), vec![IdleAction::ScreenOn]);
        assert!(!m.is_screen_off());
        // Vuelve a ociar: apaga de nuevo (rearmado).
        assert!(m.tick(4_000, false).is_empty());
        assert_eq!(m.tick(2_000, false), vec![IdleAction::ScreenOff]);
    }

    #[test]
    fn actividad_sin_pantalla_apagada_no_emite_screen_on() {
        let mut m = IdleManager::new(cfg(0, 10));
        m.tick(3_000, false);
        assert!(m.activity().is_empty()); // no había nada que encender
    }

    #[test]
    fn multimedia_inhibe_apagado_y_bloqueo() {
        let mut m = IdleManager::new(cfg(5, 10));
        // Aunque pase tiempo de sobra, con inhibidor activo no acumula.
        assert!(m.tick(60_000, true).is_empty());
        assert!(m.tick(60_000, true).is_empty());
        assert!(!m.is_screen_off());
        // Termina el vídeo: el contador parte de cero, no salta de golpe.
        assert!(m.tick(4_000, false).is_empty());
        assert_eq!(m.tick(2_000, false), vec![IdleAction::ScreenOff]);
    }

    #[test]
    fn inhibidor_ignorado_si_no_se_respeta() {
        let mut m = IdleManager::new(IdleConfig {
            screen_off_secs: 5,
            lock_secs: 0,
            respect_inhibitors: false,
        });
        // Con respeto apagado, el inhibidor no frena nada.
        assert_eq!(m.tick(6_000, true), vec![IdleAction::ScreenOff]);
    }

    #[test]
    fn inhibidor_que_aparece_con_pantalla_apagada_la_enciende() {
        let mut m = IdleManager::new(cfg(5, 0));
        m.tick(6_000, false); // apaga
        // Arranca un vídeo (inhibidor) sin tocar el teclado: cuenta como
        // actividad → enciende.
        assert_eq!(m.tick(1_000, true), vec![IdleAction::ScreenOn]);
    }
}
