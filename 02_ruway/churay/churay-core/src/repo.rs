//! `RemoteRepo` — el repositorio remoto firmado que cierra el actualizador.
//!
//! hammer no tiene índice remoto ni "buscar actualizaciones"; churay sí. Un
//! repo es una URL base que sirve:
//!   * `manifest.signed.json` — el [`SignedManifest`] (índice + firma ed25519).
//!   * `blobs/<hex>` — cada binario, **direccionado por su BLAKE3** (el `hex`
//!     es `bin_hash` sin el prefijo `b3:`). Content-addressed: el mismo binario
//!     se cachea una vez y su integridad se verifica al bajarlo.
//!
//! El transporte se abstrae en [`Fetcher`] para poder testear sin red (y para
//! soportar `file://`): producción usa [`CurlFetcher`] (curl, ubicuo en Linux);
//! los tests usan [`LocalFetcher`] (lee de un dir local).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::hash::ArtifactHash;
use crate::install::{InstallError, Source, Step};
use crate::manifest::{Manifest, SignedManifest, Unit, VerifyError};

/// Por qué falló una operación contra el repo remoto.
#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("descarga falló: {0}")]
    Fetch(String),
    #[error("el manifiesto remoto no parsea")]
    BadManifest,
    #[error("verificación del manifiesto: {0}")]
    Verify(#[from] VerifyError),
    #[error("el blob de '{program}' no coincide (esperado {esperado}, obtuvo {obtuvo})")]
    HashNoCoincide {
        program: String,
        esperado: String,
        obtuvo: String,
    },
    #[error("la unidad '{0}' no declara hash en el manifiesto remoto")]
    SinHash(String),
}

/// Transporte: cómo se traen bytes de una URL. `Send + Sync` para usarse desde
/// el worker de instalación.
pub trait Fetcher: Send + Sync {
    fn get(&self, url: &str) -> Result<Vec<u8>, RepoError>;
}

/// Transporte de producción: shell-out a `curl` (sin agregar deps pesadas;
/// curl está en cualquier Linux). `-fsSL`: falla en HTTP≥400, silencioso,
/// sigue redirecciones.
pub struct CurlFetcher;

impl Fetcher for CurlFetcher {
    fn get(&self, url: &str) -> Result<Vec<u8>, RepoError> {
        let out = std::process::Command::new("curl")
            .args(["-fsSL", "--", url])
            .output()
            .map_err(|e| RepoError::Fetch(format!("no se pudo ejecutar curl: {e}")))?;
        if !out.status.success() {
            return Err(RepoError::Fetch(format!(
                "curl {url} → {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(out.stdout)
    }
}

/// Transporte local: mapea `<base_url>/<rel>` a `<root>/<rel>` en disco. Sirve
/// para tests y para repos `file://` montados localmente.
pub struct LocalFetcher {
    pub base_url: String,
    pub root: PathBuf,
}

impl Fetcher for LocalFetcher {
    fn get(&self, url: &str) -> Result<Vec<u8>, RepoError> {
        let rel = url
            .strip_prefix(&self.base_url)
            .ok_or_else(|| RepoError::Fetch(format!("url fuera de la base: {url}")))?
            .trim_start_matches('/');
        std::fs::read(self.root.join(rel)).map_err(|e| RepoError::Fetch(e.to_string()))
    }
}

fn base(url: &str) -> &str {
    url.trim_end_matches('/')
}

/// Baja y **verifica** el manifiesto firmado del repo. Si `trusted` es `Some`,
/// además exige que la firma sea de esa clave anclada. Devuelve el `Manifest`
/// (ya verificado) listo para comparar contra lo instalado.
pub fn fetch_signed_manifest(
    base_url: &str,
    fetcher: &dyn Fetcher,
    trusted: Option<&[u8; 32]>,
) -> Result<Manifest, RepoError> {
    let url = format!("{}/manifest.signed.json", base(base_url));
    let bytes = fetcher.get(&url)?;
    let txt = String::from_utf8(bytes).map_err(|_| RepoError::BadManifest)?;
    let signed = SignedManifest::from_json(&txt).ok_or(RepoError::BadManifest)?;
    signed.verify(trusted)?;
    Ok(signed.manifest)
}

/// El repo remoto como [`Source`]: baja el binario de una unidad por su hash,
/// lo cachea y lo verifica.
pub struct RemoteRepo {
    pub base_url: String,
    /// Dónde se guardan los blobs descargados (CAS local).
    pub cache_dir: PathBuf,
    pub fetcher: Arc<dyn Fetcher>,
}

impl RemoteRepo {
    /// Repo de producción (curl).
    pub fn curl(base_url: impl Into<String>, cache_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_url: base_url.into(),
            cache_dir: cache_dir.into(),
            fetcher: Arc::new(CurlFetcher),
        }
    }

    fn blob_url(&self, hex: &str) -> String {
        format!("{}/blobs/{}", base(&self.base_url), hex)
    }

    /// Trae el blob de `expected` a la caché (si no está) verificando su hash, y
    /// devuelve la ruta cacheada.
    fn ensure_blob(&self, program: &str, expected: &ArtifactHash) -> Result<PathBuf, RepoError> {
        let hex = expected.as_str().strip_prefix("b3:").unwrap_or(expected.as_str());
        let cached = self.cache_dir.join(hex);
        if cached.exists() {
            // Confiamos en la caché direccionada por contenido (el nombre es el
            // hash). Verificación completa ya ocurrió al escribirla.
            return Ok(cached);
        }
        let bytes = self.fetcher.get(&self.blob_url(hex))?;
        let got = ArtifactHash::of_bytes(&bytes);
        if &got != expected {
            return Err(RepoError::HashNoCoincide {
                program: program.to_string(),
                esperado: expected.to_string(),
                obtuvo: got.to_string(),
            });
        }
        std::fs::create_dir_all(&self.cache_dir).map_err(|e| RepoError::Fetch(e.to_string()))?;
        // Escritura atómica de la caché.
        let tmp = cached.with_extension("tmp-dl");
        std::fs::write(&tmp, &bytes).map_err(|e| RepoError::Fetch(e.to_string()))?;
        std::fs::rename(&tmp, &cached).map_err(|e| RepoError::Fetch(e.to_string()))?;
        Ok(cached)
    }
}

impl Source for RemoteRepo {
    fn provide(
        &self,
        unit: &Unit,
        dest: &Path,
        on: &mut dyn FnMut(Step, f32),
    ) -> Result<ArtifactHash, InstallError> {
        on(Step::Descargando, 0.0);
        let expected = unit
            .bin_hash
            .as_ref()
            .ok_or_else(|| InstallError::Descarga(RepoError::SinHash(unit.id.clone()).to_string()))?;
        let cached = self
            .ensure_blob(&unit.program, expected)
            .map_err(|e| InstallError::Descarga(e.to_string()))?;
        on(Step::Copiando, 0.8);
        let hash = crate::install::atomic_install_bin(&cached, dest)?;
        on(Step::Copiando, 1.0);
        Ok(hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::{InstallConfig, InstallMode};
    use crate::manifest::{Manifest, Scope};

    fn unit(program: &str, hash: ArtifactHash) -> Unit {
        Unit {
            id: program.into(),
            label: program.into(),
            version: "2026.07".into(),
            category: "ruway".into(),
            icon: "≡".into(),
            description: "demo".into(),
            program: program.into(),
            scope: Scope::App,
            suggests: Vec::new(),
            bin_hash: Some(hash),
            size_bytes: None,
        }
    }

    /// Monta un "repo" en disco (manifest firmado + blobs por hash) y lo sirve
    /// con LocalFetcher: baja+verifica el manifiesto y luego instala una unidad
    /// desde el blob remoto. Cierra el lazo remoto sin red.
    #[test]
    fn instala_desde_repo_remoto_firmado() {
        let base = std::env::temp_dir().join(format!("churay-repo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let repo = base.join("repo");
        std::fs::create_dir_all(repo.join("blobs")).unwrap();

        // Binario "publicado" + su blob direccionado por hash.
        let contenido = b"binario remoto v2";
        let h = ArtifactHash::of_bytes(contenido);
        let hex = h.as_str().strip_prefix("b3:").unwrap();
        std::fs::write(repo.join("blobs").join(hex), contenido).unwrap();

        // Manifiesto firmado con una clave anclada.
        let kp = agora_core::Keypair::from_seed([5u8; 32]);
        let manifest = Manifest::new("2026.07", vec![unit("demo-app", h.clone())]);
        std::fs::write(repo.join("manifest.signed.json"), manifest.sign(&kp).to_json()).unwrap();

        let base_url = "mem://repo".to_string();
        let fetcher = LocalFetcher { base_url: base_url.clone(), root: repo.clone() };

        // 1) bajar + verificar el manifiesto contra la clave anclada.
        let remoto = fetch_signed_manifest(&base_url, &fetcher, Some(&kp.public_key()))
            .expect("manifiesto remoto verifica");
        assert_eq!(remoto.units.len(), 1);

        // 2) instalar la unidad desde el repo remoto (Source).
        let cfg = InstallConfig {
            mode: InstallMode::Local,
            prefix: base.join("prefix"),
            bundle_dir: None,
            workspace_root: None,
            remote_base_url: None,
            cache_dir: base.join("cache"),
        };
        let src = RemoteRepo {
            base_url,
            cache_dir: cfg.cache_dir.clone(),
            fetcher: Arc::new(fetcher),
        };
        let dest = cfg.prefix.join("bin").join("demo-app");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let got = src.provide(remoto.get("demo-app").unwrap(), &dest, &mut |_, _| {}).unwrap();
        assert_eq!(got, h, "hash instalado = hash publicado");
        assert!(dest.exists());
        // El blob quedó cacheado por su hash (CAS local).
        assert!(cfg.cache_dir.join(hex).exists());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn manifiesto_remoto_de_otra_clave_se_rechaza() {
        let base = std::env::temp_dir().join(format!("churay-repo-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let firmante = agora_core::Keypair::from_seed([5u8; 32]);
        let manifest = Manifest::new("2026.07", vec![]);
        std::fs::write(repo.join("manifest.signed.json"), manifest.sign(&firmante).to_json()).unwrap();

        let base_url = "mem://repo".to_string();
        let fetcher = LocalFetcher { base_url: base_url.clone(), root: repo };
        let otra = agora_core::Keypair::from_seed([9u8; 32]).public_key();
        let err = fetch_signed_manifest(&base_url, &fetcher, Some(&otra)).unwrap_err();
        assert!(matches!(err, RepoError::Verify(VerifyError::Untrusted)));
        let _ = std::fs::remove_dir_all(&base);
    }
}
