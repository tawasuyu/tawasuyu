//! `brahman-card-wit` — extractor de contratos WIT.
//!
//! Crate **opcional** (no es dep de `brahman-card`). Parsea texto WIT
//! mediante [`wit-parser`] y devuelve una lista de [`WitInterface`]
//! (uno por `world`) lista para acoplarse a una [`card_core::Card`]
//! cuando se construye una [`card_core::ResolvedCard`].
//!
//! Casos de uso:
//!
//! - El Init lee `<modulo>/wit/protocol.wit` durante el descubrimiento
//!   y lo combina con la Card del módulo para obtener una
//!   `ResolvedCard::from_conscious(card, wit)`.
//! - Tooling (`brahman-wit-info`) inspecciona un `.wit` y muestra
//!   sus mundos, exports e imports.
//!
//! No depende de `wasm-tools`/`wit-component` — sólo del parser texto.

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

use std::path::{Path, PathBuf};

use card_core::WitInterface;
use thiserror::Error;
use wit_parser::{Resolve, WorldKey};

#[derive(Debug, Error)]
pub enum WitError {
    #[error("parse: {0}")]
    Parse(String),
    #[error("E/S: {0}")]
    Io(#[from] std::io::Error),
}

/// Parsea WIT desde una string. Devuelve un `WitInterface` por cada
/// `world` declarado.
pub fn parse_wit(source: &str) -> Result<Vec<WitInterface>, WitError> {
    parse_with_path(source, Path::new("inline.wit"))
}

/// Parsea WIT desde un archivo. Útil para `module/wit/protocol.wit`.
pub fn parse_wit_file(path: impl AsRef<Path>) -> Result<Vec<WitInterface>, WitError> {
    let p = path.as_ref();
    let source = std::fs::read_to_string(p)?;
    parse_with_path(&source, p)
}

fn parse_with_path(source: &str, path: &Path) -> Result<Vec<WitInterface>, WitError> {
    let mut resolve = Resolve::new();
    let path_buf: PathBuf = path.to_path_buf();
    resolve
        .push_str(&path_buf, source)
        .map_err(|e| WitError::Parse(e.to_string()))?;

    let mut out = Vec::new();
    for (_pkg_id, pkg) in resolve.packages.iter() {
        let pkg_name = pkg.name.to_string();
        for (_name, &world_id) in &pkg.worlds {
            let world = &resolve.worlds[world_id];
            let exports = collect_keys(world.exports.iter().map(|(k, _)| k), &resolve);
            let imports = collect_keys(world.imports.iter().map(|(k, _)| k), &resolve);
            out.push(WitInterface {
                package: pkg_name.clone(),
                world: world.name.clone(),
                exports,
                imports,
            });
        }
    }
    Ok(out)
}

fn collect_keys<'a, I>(keys: I, resolve: &Resolve) -> Vec<String>
where
    I: Iterator<Item = &'a WorldKey>,
{
    keys.map(|k| match k {
        WorldKey::Name(n) => n.clone(),
        WorldKey::Interface(id) => resolve.interfaces[*id]
            .name
            .clone()
            .unwrap_or_else(|| format!("<interface#{}>", id.index())),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
package brahman:test@0.1.0;

interface handshake {
    hello: func() -> result<_, string>;
}

interface lifecycle {
    report: func();
}

world module {
    import handshake;
    import lifecycle;
    export run: func() -> result<_, string>;
}
"#;

    #[test]
    fn parses_inline_wit() {
        let worlds = parse_wit(SAMPLE).unwrap();
        assert_eq!(worlds.len(), 1, "esperaba un único world");
        let w = &worlds[0];
        assert!(w.package.starts_with("brahman:test"));
        assert_eq!(w.world, "module");
        assert!(
            w.imports.iter().any(|i| i == "handshake"),
            "imports={:?}",
            w.imports
        );
        assert!(
            w.imports.iter().any(|i| i == "lifecycle"),
            "imports={:?}",
            w.imports
        );
        assert!(
            w.exports.iter().any(|e| e == "run"),
            "exports={:?}",
            w.exports
        );
    }

    #[test]
    fn parses_shared_protocol() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../shared_wit/protocol.wit");
        let worlds = parse_wit_file(path).unwrap();
        assert!(
            worlds.iter().any(|w| w.world == "module"),
            "no encontró world 'module' en {:?}",
            worlds.iter().map(|w| &w.world).collect::<Vec<_>>()
        );
        assert!(
            worlds.iter().any(|w| w.world == "admin-host"),
            "no encontró world 'admin-host'"
        );
    }

    #[test]
    fn parse_error_on_garbage() {
        let bad = "this is not wit at all { } } ;;;;";
        assert!(matches!(parse_wit(bad), Err(WitError::Parse(_))));
    }

    #[test]
    fn empty_world_handled() {
        let src = r#"
package brahman:empty@0.1.0;
world hollow {}
"#;
        let worlds = parse_wit(src).unwrap();
        assert_eq!(worlds.len(), 1);
        assert!(worlds[0].exports.is_empty());
        assert!(worlds[0].imports.is_empty());
    }
}
