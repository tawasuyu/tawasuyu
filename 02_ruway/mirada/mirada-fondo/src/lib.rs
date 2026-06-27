//! `mirada-fondo` — el sistema **unificado** de fondos animados de las tres
//! superficies de presentación de la suite:
//!
//! - **splash** (`arje-splash`) — arranque, sin GPU: blitea bytes crudos.
//! - **greeter** (`mirada-greeter`) — app vello: pinta a un `Scene` en vivo.
//! - **wallpaper** (`mirada-compositor`) — sube bytes a la GPU.
//!
//! Las tres comparten un mismo [`FondoSpec`] y, por **defecto**, la misma
//! **chakana animada de la marca** ([`FondoSpec::Chakana`]). Sobre ese default,
//! el usuario puede elegir un Lottie (`.json`) o un proyecto «rive»
//! (`llimphi-anim-studio`, `.ron` con `Doc` + `RigDoc` + assets).
//!
//! ## Por qué un crate y no lógica triplicada
//!
//! Cada superficie pinta sobre un substrato distinto (framebuffer crudo, `Scene`
//! vello, textura GPU), pero la *fuente* del fondo es la misma. Este crate
//! centraliza:
//!
//! - El [`FondoSpec`] serializable que cada config guarda (en su propio formato:
//!   `greeter.conf`, `splash.conf`, `wallpaper.ron` — sin unificar el formato,
//!   sí el significado).
//! - La **chakana** como bytes CPU ([`chakana_frame`]) — sirve a las tres sin
//!   vello (es `marca::animated_frame`).
//! - La **cache de frames** ([`cache`]): Lottie/rive se *bakean* una vez a una
//!   secuencia de PNG (feature `bake`, render headless vello) y las superficies
//!   GPU-less (splash, compositor) **bliteant** esos frames. El greeter, que sí
//!   tiene vello, los reproduce en vivo sin cache.
//!
//! ## El muro que esto resuelve
//!
//! `vello 0.7` es GPU-only: no hay rasterizador CPU de un `Scene`. Splash y
//! compositor no corren vello, así que no pueden pintar Lottie/rive directo. El
//! *bake* headless (que sí abre una GPU una sola vez, offline) produce frames que
//! cualquiera blitea. La chakana no sufre esto: es píxeles CPU desde el vamos.

#![forbid(unsafe_code)]

pub mod cache;

use serde::{Deserialize, Serialize};

/// Qué fondo mostrar en una superficie. El **default** es la chakana de la marca.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FondoSpec {
    /// La chakana animada de `marca` — procedural en CPU, sin assets ni cache.
    /// Es el fondo por defecto de las tres superficies.
    Chakana,
    /// Un archivo Lottie (`.json`), reproducido por `llimphi-lottie`.
    Lottie {
        /// Ruta absoluta al `.json` Lottie.
        path: String,
    },
    /// Un proyecto «rive»: el `.ron` que guarda `llimphi-anim-studio`
    /// (`Project { doc, rig }`) más sus assets (textura del rig / clips Lottie),
    /// referenciados por ruta relativa al `.ron`.
    Rive {
        /// Ruta absoluta al `.ron` del proyecto de animación.
        path: String,
    },
}

impl Default for FondoSpec {
    fn default() -> Self {
        FondoSpec::Chakana
    }
}

impl FondoSpec {
    /// Etiqueta corta y estable (la que guardan las configs `clave = valor`).
    pub fn kind(&self) -> &'static str {
        match self {
            FondoSpec::Chakana => "chakana",
            FondoSpec::Lottie { .. } => "lottie",
            FondoSpec::Rive { .. } => "rive",
        }
    }

    /// La ruta del asset, si la hay (Lottie/rive). `Chakana` no tiene.
    pub fn path(&self) -> Option<&str> {
        match self {
            FondoSpec::Chakana => None,
            FondoSpec::Lottie { path } | FondoSpec::Rive { path } => Some(path.as_str()),
        }
    }

    /// Reconstruye un spec desde `(kind, path)` — el par que persisten las configs
    /// de texto plano (`source = lottie`, `lottie = /ruta`). Un `kind`
    /// desconocido o un Lottie/rive sin ruta caen a [`FondoSpec::Chakana`], que es
    /// el default seguro que siempre renderiza.
    pub fn from_parts(kind: &str, path: Option<&str>) -> Self {
        match (kind.trim(), path.map(str::trim).filter(|s| !s.is_empty())) {
            ("lottie", Some(p)) => FondoSpec::Lottie { path: p.to_string() },
            ("rive", Some(p)) => FondoSpec::Rive { path: p.to_string() },
            _ => FondoSpec::Chakana,
        }
    }

    /// ¿Necesita una cache de frames bakeada para las superficies GPU-less?
    /// (Lottie/rive sí; la chakana no.)
    pub fn needs_bake(&self) -> bool {
        !matches!(self, FondoSpec::Chakana)
    }
}

/// Duración del loop de la chakana animada (segundos). Espeja `marca`'s `LOOP_SECS`:
/// el frame en `t` y en `t + CHAKANA_LOOP_SECS` son idénticos byte-a-byte.
pub const CHAKANA_LOOP_SECS: f32 = 24.0;

/// Un frame de la chakana animada en el instante `t` (segundos), tamaño `w×h`.
/// Devuelve bytes **BGRA** opacos, listos para blitear a un framebuffer XRGB8888
/// o subir a la GPU. Es exactamente `marca::animated_frame` — el mismo glifo que
/// ya pinta el compositor por defecto, ahora compartido por las tres superficies.
pub fn chakana_frame(t: f32, w: u32, h: u32) -> Vec<u8> {
    marca::animated_frame(t, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_es_chakana() {
        assert_eq!(FondoSpec::default(), FondoSpec::Chakana);
        assert!(!FondoSpec::default().needs_bake());
    }

    #[test]
    fn from_parts_redondea_a_seguro() {
        assert_eq!(
            FondoSpec::from_parts("lottie", Some("/x.json")),
            FondoSpec::Lottie { path: "/x.json".into() }
        );
        assert_eq!(
            FondoSpec::from_parts("rive", Some("/p.ron")),
            FondoSpec::Rive { path: "/p.ron".into() }
        );
        // kind válido pero sin ruta → chakana (no hay nada que reproducir).
        assert_eq!(FondoSpec::from_parts("lottie", None), FondoSpec::Chakana);
        assert_eq!(FondoSpec::from_parts("lottie", Some("  ")), FondoSpec::Chakana);
        // kind desconocido → chakana.
        assert_eq!(FondoSpec::from_parts("ruido", Some("/x")), FondoSpec::Chakana);
        // chakana explícita.
        assert_eq!(FondoSpec::from_parts("chakana", None), FondoSpec::Chakana);
    }

    #[test]
    fn round_trip_kind_path() {
        for spec in [
            FondoSpec::Chakana,
            FondoSpec::Lottie { path: "/a/b.json".into() },
            FondoSpec::Rive { path: "/a/p.ron".into() },
        ] {
            let back = FondoSpec::from_parts(spec.kind(), spec.path());
            assert_eq!(spec, back, "ida y vuelta por (kind,path)");
        }
    }

    #[test]
    fn la_chakana_es_bgra_opaca_del_tamano_pedido() {
        let f = chakana_frame(0.0, 8, 4);
        assert_eq!(f.len(), 8 * 4 * 4, "BGRA = 4 bytes/píxel");
        // alpha opaco en todos los píxeles.
        assert!(f.chunks_exact(4).all(|px| px[3] == 255));
    }

    #[test]
    fn la_chakana_cierra_el_loop() {
        // El contrato de marca: t y t+LOOP son idénticos.
        let a = chakana_frame(1.5, 16, 16);
        let b = chakana_frame(1.5 + CHAKANA_LOOP_SECS, 16, 16);
        assert_eq!(a, b, "la chakana debe cerrar el loop a {CHAKANA_LOOP_SECS}s");
    }
}
