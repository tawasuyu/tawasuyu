//! El manifest `.ron` que declara un plugin: su `.wasm`, su tipo y las
//! capacidades que pide. Las capacidades aquí son una **declaración**; el host
//! las verifica contra las importaciones reales del módulo al cargar
//! (fail-closed, ver [`crate::wasm`]).

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::caps::{parse_cap, CapsPlugin};
use crate::trust::{parse_pubkey, Grant};

/// El tipo de plugin determina qué export busca el host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum PluginKind {
    /// Exporta `mirada_tile` — decide la geometría del teselado.
    Layout,
    /// Exporta `mirada_on_event` — reacciona a eventos y emite comandos.
    Reactor,
}

/// El manifest tal cual se deserializa del `.ron`.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    /// Ruta al `.wasm`, relativa al directorio del manifest (o absoluta).
    pub wasm: String,
    /// Tipo de plugin.
    pub kind: PluginKind,
    /// Capacidades pedidas, por nombre (`"layout"`, `"spawn"`, …).
    #[serde(default)]
    pub caps: Vec<String>,
    /// Prioridad de arbitraje (mayor gana el rol singleton de layout).
    #[serde(default)]
    pub priority: i32,
    /// Clave pública del firmante (`"ed25519:hex64"`). Requerida si el plugin
    /// pide capacidades peligrosas (cualquiera más allá de `layout`). Vacía =
    /// sin firma. (String y no Option: RON pide bare-strings, no `Some(...)`.)
    #[serde(default)]
    pub signer: String,
    /// Firma Ed25519 (hex de 64 bytes) sobre `blake3(wasm) ‖ caps`. Vacía = sin
    /// firma.
    #[serde(default)]
    pub signature: String,
}

/// Un manifest ya resuelto: ruta absoluta del `.wasm` + bitfield de capacidades.
#[derive(Debug, Clone)]
pub struct ResolvedManifest {
    pub wasm_path: PathBuf,
    pub kind: PluginKind,
    pub granted: CapsPlugin,
    pub priority: i32,
    /// Nombre legible (el del archivo del manifest), para logs/errores.
    pub name: String,
    /// La firma del grant, si el manifest la trae.
    pub grant: Option<Grant>,
}

impl PluginManifest {
    /// Carga y resuelve un manifest desde un archivo `.ron`.
    pub fn load(manifest_path: &Path) -> Result<ResolvedManifest, String> {
        let text = std::fs::read_to_string(manifest_path)
            .map_err(|e| format!("no se pudo leer {}: {e}", manifest_path.display()))?;
        let m: PluginManifest = ron::from_str(&text)
            .map_err(|e| format!("manifest {} inválido: {e}", manifest_path.display()))?;

        let base = manifest_path.parent().unwrap_or(Path::new("."));
        let wasm_path = {
            let p = Path::new(&m.wasm);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                base.join(p)
            }
        };

        let mut granted: CapsPlugin = 0;
        for c in &m.caps {
            match parse_cap(c) {
                Some(bit) => granted |= bit,
                None => return Err(format!("capacidad desconocida en el manifest: {c:?}")),
            }
        }

        let name = manifest_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "plugin".to_string());

        let grant = parse_grant(&m)?;

        Ok(ResolvedManifest {
            wasm_path,
            kind: m.kind,
            granted,
            priority: m.priority,
            name,
            grant,
        })
    }
}

/// Resuelve los campos `signer`/`signature` del manifest a un [`Grant`].
/// Ambos o ninguno; presencia parcial es un error.
fn parse_grant(m: &PluginManifest) -> Result<Option<Grant>, String> {
    let (s, sig) = (m.signer.trim(), m.signature.trim());
    match (s.is_empty(), sig.is_empty()) {
        (true, true) => Ok(None),
        (false, false) => {
            let signer = parse_pubkey(s).ok_or_else(|| format!("`signer` ilegible: {s:?}"))?;
            let bytes = hex::decode(sig).map_err(|e| format!("`signature` no es hex: {e}"))?;
            let signature: [u8; 64] = bytes
                .try_into()
                .map_err(|_| "`signature` no mide 64 bytes".to_string())?;
            Ok(Some(Grant { signer, signature }))
        }
        _ => Err("el manifest trae `signer` o `signature` pero no ambos".to_string()),
    }
}
