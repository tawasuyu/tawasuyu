//! `pata-config` — el loader del marco en Linux.
//!
//! `pata-core` es `no_std` y no sabe leer archivos; este crate es el puente al
//! disco: busca el TOML del usuario en las rutas XDG, lo parsea al modelo y, si
//! no hay nada, cae al [`Config::preset`]. En wawa este rol lo cumple akasha
//! —el config llega direccionado por contenido—, no este crate.

use std::path::PathBuf;

pub use pata_core::{layout::resolve, Config, Frame, Rect};

/// Las rutas donde se busca `launcher.toml`, en orden de prioridad:
/// `$XDG_CONFIG_HOME/pata/` y luego `$HOME/.config/pata/`.
pub fn candidate_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        out.push(PathBuf::from(xdg).join("pata/launcher.toml"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        out.push(PathBuf::from(home).join(".config/pata/launcher.toml"));
    }
    out
}

/// Parsea un TOML al modelo. Error con el detalle de toml si no cuadra.
pub fn load_from_str(src: &str) -> Result<Config, toml::de::Error> {
    toml::from_str(src)
}

/// Carga el marco: el primer `launcher.toml` que parsee gana; si ninguno
/// existe o todos fallan, devuelve el [`Config::preset`]. Diagnostica por
/// stderr cuál cargó o por qué cayó al default.
pub fn load() -> Config {
    for path in candidate_paths() {
        match std::fs::read_to_string(&path) {
            Ok(text) => match load_from_str(&text) {
                Ok(cfg) => {
                    eprintln!("pata · cargué {}", path.display());
                    return cfg;
                }
                Err(e) => {
                    eprintln!("pata · {} no parsea ({e}); intento siguiente", path.display());
                }
            },
            Err(_) => { /* no existe en esta ruta; sigo */ }
        }
    }
    eprintln!("pata · sin launcher.toml; uso el preset");
    Config::preset()
}

/// Serializa el marco a TOML. Lo expone aparte de [`save`] para testear el
/// round-trip sin tocar el disco.
pub fn to_toml(cfg: &Config) -> Result<String, toml::ser::Error> {
    toml::to_string_pretty(cfg)
}

/// Persiste el marco al `launcher.toml` del usuario (la primera ruta candidata:
/// `$XDG_CONFIG_HOME/pata/` o `$HOME/.config/pata/`). Escribe atómicamente
/// (tmp + rename) y crea el directorio si falta. Devuelve la ruta escrita.
///
/// Es el camino inverso de [`load`]: lo usa el panel de configuración para
/// guardar lo que el usuario edita en la UI, sin que tenga que tocar el archivo.
pub fn save(cfg: &Config) -> std::io::Result<PathBuf> {
    let path = candidate_paths().into_iter().next().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "ni XDG_CONFIG_HOME ni HOME definidos",
        )
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text =
        to_toml(cfg).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pata_core::{Anchor, SurfaceKind};

    #[test]
    fn load_from_str_parsea_dos_superficies() {
        let cfg = load_from_str(
            r#"
            [[surfaces]]
            anchor = "top"
            thickness = 30

            [[surfaces.start]]
            kind = "clock"

            [[surfaces]]
            kind = "dock"
            anchor = "bottom"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.surfaces.len(), 2);
        assert_eq!(cfg.surfaces[0].anchor, Anchor::Top);
        assert_eq!(cfg.surfaces[0].start[0].kind, "clock");
        assert_eq!(cfg.surfaces[1].kind, SurfaceKind::Dock);
    }

    #[test]
    fn candidate_paths_respeta_xdg() {
        // No tocamos el entorno global: sólo verificamos que la función
        // produce rutas terminadas en pata/launcher.toml cuando hay HOME.
        let paths = candidate_paths();
        assert!(paths.iter().all(|p| p.ends_with("pata/launcher.toml")));
    }

    #[test]
    fn toml_invalido_es_error_no_panic() {
        assert!(load_from_str("esto no es toml [[[").is_err());
    }

    #[test]
    fn round_trip_preset_serializa_y_reparsea() {
        // El preset trae barras con slots (arrays de tablas) + escalares: el
        // caso que estresa el orden "valores antes que tablas" de toml.
        let cfg = Config::preset();
        let text = to_toml(&cfg).expect("preset debe serializar a TOML");
        let back = load_from_str(&text).expect("el TOML serializado debe reparsear");
        assert_eq!(cfg, back);
    }

    #[test]
    fn round_trip_sidebar_con_dientes() {
        use pata_core::{Prop, SidebarTab, Surface, WidgetSpec};
        let mut cfg = Config::default();
        let mut sb = Surface::sidebar(Anchor::Left);
        sb.tabs.push(SidebarTab::new(
            "monads",
            "Mónadas",
            WidgetSpec::new("navigator").with("source", Prop::Str("nouser".into())),
        ));
        cfg.surfaces.push(sb);
        let text = to_toml(&cfg).expect("sidebar debe serializar");
        let back = load_from_str(&text).expect("debe reparsear");
        assert_eq!(cfg, back);
    }
}
