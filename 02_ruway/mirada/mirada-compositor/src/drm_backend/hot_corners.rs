//! **Esquinas calientes** (hot corners): al apoyar el puntero en una de las 8
//! zonas del borde de una salida (4 esquinas + 4 centros de lado), tras un
//! reposo breve se dispara la acción configurada (ver [`mirada_brain::HotCorners`]),
//! con un resplandor que crece como aviso (lo pinta `render::emit_hot_corners`).
//!
//! El reparto de responsabilidades:
//!   - [`zone_at`] (pura, testeada) decide en qué zona cae el puntero.
//!   - [`DrmState::update_hot_corners`] (en el motion de input) arma/rearma la
//!     zona bajo el cursor y sella el instante de entrada.
//!   - [`DrmState::tick_hot_corners`] (en el `tick` ~60 Hz) dispara cuando se
//!     cumple el reposo y rearma sólo al salir de la zona.
//!   - [`DrmState::fire_hot_action`] traduce la cadena de acción a su efecto.

use super::DrmState;
use mirada_brain::HotCorners;

/// Las 8 zonas sensibles del borde de una salida.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HotZone {
    TopLeft,
    TopCenter,
    TopRight,
    RightCenter,
    BottomRight,
    BottomCenter,
    BottomLeft,
    LeftCenter,
}

/// El estado vivo de la esquina caliente bajo el puntero: qué zona, en qué
/// salida, desde cuándo (ms del reloj del Cuerpo) y si ya disparó (se rearma
/// recién al salir de la zona, no se redispara mientras el puntero siga dentro).
#[derive(Debug, Clone, Copy)]
pub(crate) struct HotState {
    pub zone: HotZone,
    pub output: usize,
    pub since_ms: u32,
    pub fired: bool,
}

/// La acción asignada a `zone` en `cfg` (cadena vacía/`"none"` = inerte).
pub(crate) fn action_for(cfg: &HotCorners, zone: HotZone) -> &str {
    match zone {
        HotZone::TopLeft => &cfg.top_left,
        HotZone::TopCenter => &cfg.top_center,
        HotZone::TopRight => &cfg.top_right,
        HotZone::RightCenter => &cfg.right_center,
        HotZone::BottomRight => &cfg.bottom_right,
        HotZone::BottomCenter => &cfg.bottom_center,
        HotZone::BottomLeft => &cfg.bottom_left,
        HotZone::LeftCenter => &cfg.left_center,
    }
}

/// `true` si la acción es inerte (`""`/`"none"`): esas zonas ni se arman ni brillan.
pub(crate) fn is_inert(action: &str) -> bool {
    action.is_empty() || action == "none"
}

/// En qué zona caen las coords **locales** `(lx, ly)` dentro de una salida de
/// `w×h`, con zona sensible de `size` px. Las esquinas tienen prioridad sobre
/// los centros de lado; el resto del borde no dispara. Pura — la base de los
/// tests. `None` fuera de toda zona o con dimensiones/size degenerados.
pub(crate) fn zone_at(w: i32, h: i32, lx: i32, ly: i32, size: i32) -> Option<HotZone> {
    if w <= 0 || h <= 0 || size <= 0 || lx < 0 || ly < 0 || lx >= w || ly >= h {
        return None;
    }
    // La zona no puede pasar de la mitad del eje (en pantallas diminutas).
    let s = size.min(w / 2).min(h / 2).max(1);
    let near_l = lx < s;
    let near_r = lx >= w - s;
    let near_t = ly < s;
    let near_b = ly >= h - s;
    // Esquinas (prioridad sobre los centros).
    if near_t && near_l {
        return Some(HotZone::TopLeft);
    }
    if near_t && near_r {
        return Some(HotZone::TopRight);
    }
    if near_b && near_l {
        return Some(HotZone::BottomLeft);
    }
    if near_b && near_r {
        return Some(HotZone::BottomRight);
    }
    // Centros de lado: la banda central del borde (un cuarto del lado, mínimo
    // 4× la zona, para que sea alcanzable sin invadir las esquinas).
    let band_h = (w / 4).max(s * 4).min(w);
    let band_v = (h / 4).max(s * 4).min(h);
    let in_hmid = lx >= (w - band_h) / 2 && lx < (w + band_h) / 2;
    let in_vmid = ly >= (h - band_v) / 2 && ly < (h + band_v) / 2;
    if near_t && in_hmid {
        return Some(HotZone::TopCenter);
    }
    if near_b && in_hmid {
        return Some(HotZone::BottomCenter);
    }
    if near_l && in_vmid {
        return Some(HotZone::LeftCenter);
    }
    if near_r && in_vmid {
        return Some(HotZone::RightCenter);
    }
    None
}

/// El rect **local** `(x, y, w, h)` del resplandor de una zona dentro de una
/// salida de `ow×oh`. Es más grande que la zona sensible (bloom), anclado al
/// borde/esquina. Pura y testeada.
pub(crate) fn glow_rect(zone: HotZone, ow: i32, oh: i32, size: i32) -> (i32, i32, i32, i32) {
    // Extensión base del resplandor (cuadrado de esquina / grosor de banda).
    let g = (size * 6).clamp(36, 180).min(ow).min(oh);
    // Largo de las bandas centrales (un tercio del lado, acotado).
    let bh = (ow / 3).clamp(g, ow);
    let bv = (oh / 3).clamp(g, oh);
    match zone {
        HotZone::TopLeft => (0, 0, g, g),
        HotZone::TopRight => (ow - g, 0, g, g),
        HotZone::BottomLeft => (0, oh - g, g, g),
        HotZone::BottomRight => (ow - g, oh - g, g, g),
        HotZone::TopCenter => ((ow - bh) / 2, 0, bh, g),
        HotZone::BottomCenter => ((ow - bh) / 2, oh - g, bh, g),
        HotZone::LeftCenter => (0, (oh - bv) / 2, g, bv),
        HotZone::RightCenter => (ow - g, (oh - bv) / 2, g, bv),
    }
}

impl DrmState {
    /// Arma/rearma la zona caliente bajo el puntero global `(x, y)`. Llamada en
    /// cada movimiento del puntero (tras `update_shell_autohide`). Sella el
    /// instante de entrada para el reposo y el resplandor; al cambiar de zona o
    /// salir, reinicia. No-op (y limpia) si la feature está apagada o hay un
    /// shell de credenciales arriba (login/lock).
    pub(super) fn update_hot_corners(&mut self, x: f64, y: f64) {
        let cfg = self.app.config_hot_corners();
        if !cfg.enabled || self.app.shell_activo() {
            self.hot_zone = None;
            return;
        }
        let xi = x.round() as i32;
        let yi = y.round() as i32;
        let idx = self.output_at_point(xi, yi);
        let Some(rect) = self.outputs.get(idx).map(|o| o.rect) else {
            self.hot_zone = None;
            return;
        };
        let zone = zone_at(rect.w, rect.h, xi - rect.x, yi - rect.y, cfg.size_px)
            // Sólo cuentan las zonas con acción asignada: las inertes ni arman ni brillan.
            .filter(|&z| !is_inert(action_for(&cfg, z)));
        match (self.hot_zone, zone) {
            // Sigue en la misma zona y salida: conservamos el sellado.
            (Some(h), Some(z)) if h.zone == z && h.output == idx => {}
            // Zona nueva (o cambió de zona/salida): re-sellamos.
            (_, Some(z)) => {
                let now = self.start.elapsed().as_millis() as u32;
                self.hot_zone = Some(HotState {
                    zone: z,
                    output: idx,
                    since_ms: now,
                    fired: false,
                });
            }
            // Salió de toda zona.
            (Some(_), None) => self.hot_zone = None,
            (None, None) => {}
        }
    }

    /// Dispara la zona caliente si el puntero cumplió el reposo. Llamada en el
    /// `tick` (~60 Hz), así el reposo se mide aunque el cursor quede quieto (sin
    /// más eventos de movimiento). Se dispara una vez por entrada (`fired`).
    pub(super) fn tick_hot_corners(&mut self) {
        let Some(h) = self.hot_zone else {
            return;
        };
        if h.fired {
            return;
        }
        let cfg = self.app.config_hot_corners();
        if !cfg.enabled {
            return;
        }
        let now = self.start.elapsed().as_millis() as u32;
        if now.saturating_sub(h.since_ms) < cfg.dwell_ms {
            return;
        }
        // Sella el disparo ANTES de actuar (la acción puede repintar/reentrar).
        if let Some(s) = self.hot_zone.as_mut() {
            s.fired = true;
        }
        let action = action_for(&cfg, h.zone).to_string();
        self.fire_hot_action(&action);
    }

    /// Traduce la cadena de acción de una esquina caliente a su efecto. Primero
    /// las pseudo-acciones que vive el Cuerpo; si no, se parsea como
    /// [`mirada_brain::DesktopAction`] y se aplica por el mismo camino del
    /// keymap/`mirada-ctl`.
    fn fire_hot_action(&mut self, action: &str) {
        match action {
            "" | "none" => {}
            "reveal-shell" => {
                self.app.reveal_shell();
            }
            "overview" => {
                self.app.open_overview();
            }
            "root-menu" => {
                self.open_root_menu_at_pointer();
            }
            other => match other.parse::<mirada_brain::DesktopAction>() {
                Ok(act) => {
                    self.app.serve_ctl(mirada_brain::CtlRequest::Do(act));
                }
                Err(e) => {
                    eprintln!("mirada-compositor · esquina caliente: acción inválida «{other}»: {e}");
                }
            },
        }
    }

    /// Abre el menú raíz en la posición del puntero (acción `root-menu`). No-op
    /// si ya hay un menú abierto o no hay salida bajo el cursor.
    fn open_root_menu_at_pointer(&mut self) {
        if self.root_menu.is_some() {
            return;
        }
        let (x, y) = self.app.pointer_loc;
        let idx = self.output_at_point(x.round() as i32, y.round() as i32);
        let Some(r) = self.outputs.get(idx).map(|o| o.rect) else {
            return;
        };
        self.menu_output_idx = Some(idx);
        self.menu_window = None;
        self.root_menu = Some(crate::menu::RootMenu::open(
            (x.round() as i32 - r.x).max(0),
            (y.round() as i32 - r.y).max(0),
            self.menu_entries.clone(),
            r.w,
            r.h,
        ));
        crate::screencopy::danar_todo(&mut self.app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn las_cuatro_esquinas_se_detectan() {
        let (w, h, s) = (1920, 1080, 8);
        assert_eq!(zone_at(w, h, 0, 0, s), Some(HotZone::TopLeft));
        assert_eq!(zone_at(w, h, w - 1, 0, s), Some(HotZone::TopRight));
        assert_eq!(zone_at(w, h, 0, h - 1, s), Some(HotZone::BottomLeft));
        assert_eq!(zone_at(w, h, w - 1, h - 1, s), Some(HotZone::BottomRight));
    }

    #[test]
    fn los_cuatro_centros_se_detectan() {
        let (w, h, s) = (1920, 1080, 8);
        assert_eq!(zone_at(w, h, w / 2, 0, s), Some(HotZone::TopCenter));
        assert_eq!(zone_at(w, h, w / 2, h - 1, s), Some(HotZone::BottomCenter));
        assert_eq!(zone_at(w, h, 0, h / 2, s), Some(HotZone::LeftCenter));
        assert_eq!(zone_at(w, h, w - 1, h / 2, s), Some(HotZone::RightCenter));
    }

    #[test]
    fn la_esquina_gana_al_centro_en_el_solape() {
        // En la franja superior, el extremo es esquina aunque también sea borde.
        let (w, h, s) = (1920, 1080, 8);
        assert_eq!(zone_at(w, h, 1, 1, s), Some(HotZone::TopLeft));
    }

    #[test]
    fn el_centro_de_la_pantalla_no_dispara() {
        assert_eq!(zone_at(1920, 1080, 960, 540, 8), None);
        // Borde superior pero fuera de la banda central: tampoco.
        assert_eq!(zone_at(1920, 1080, 200, 0, 8), None);
    }

    #[test]
    fn fuera_de_rango_es_none() {
        assert_eq!(zone_at(1920, 1080, -1, 5, 8), None);
        assert_eq!(zone_at(1920, 1080, 5, 2000, 8), None);
        assert_eq!(zone_at(0, 0, 0, 0, 8), None);
        assert_eq!(zone_at(1920, 1080, 5, 5, 0), None);
    }

    #[test]
    fn el_resplandor_de_esquina_abraza_el_origen_correcto() {
        let (ow, oh) = (1920, 1080);
        let (x, y, gw, gh) = glow_rect(HotZone::TopLeft, ow, oh, 8);
        assert_eq!((x, y), (0, 0));
        assert!(gw > 0 && gh > 0);
        let (x2, _, gw2, _) = glow_rect(HotZone::BottomRight, ow, oh, 8);
        assert_eq!(x2 + gw2, ow); // pegado al borde derecho
    }

    #[test]
    fn el_resplandor_cabe_en_la_salida() {
        let (ow, oh) = (1920, 1080);
        for z in [
            HotZone::TopLeft,
            HotZone::TopCenter,
            HotZone::TopRight,
            HotZone::RightCenter,
            HotZone::BottomRight,
            HotZone::BottomCenter,
            HotZone::BottomLeft,
            HotZone::LeftCenter,
        ] {
            let (x, y, w, h) = glow_rect(z, ow, oh, 8);
            assert!(x >= 0 && y >= 0, "{z:?} origen negativo");
            assert!(x + w <= ow && y + h <= oh, "{z:?} se sale de la salida");
        }
    }

    #[test]
    fn action_for_mapea_las_ocho_zonas() {
        let mut cfg = HotCorners::default();
        cfg.top_left = "overview".into();
        cfg.bottom_center = "reveal-shell".into();
        assert_eq!(action_for(&cfg, HotZone::TopLeft), "overview");
        assert_eq!(action_for(&cfg, HotZone::BottomCenter), "reveal-shell");
        assert!(is_inert(action_for(&cfg, HotZone::TopRight)));
    }
}
