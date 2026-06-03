// =============================================================================
//  uya-app::identidad — la identidad soberana del participante local.
// -----------------------------------------------------------------------------
//  El id de un participante (`ParticipanteId`) es la huella BLAKE3 de su clave
//  pública Ed25519 — inforjable: para presentarse con un id hay que poseer la
//  clave secreta que lo engendra. Esa clave nace de una **semilla de 32 bytes**
//  que vive persistida en disco (CSPRNG la primera vez); la MISMA semilla
//  alimenta el keypair libp2p del transporte (ver `enlace::arrancar`), de modo
//  que identidad de app y de red comparten una sola raíz secreta.
//
//  Antes el keypair derivaba de `BLAKE3(nombre)` — pero el nombre es público,
//  así que cualquiera podía re-derivar la clave de otro y suplantarlo. Ahora el
//  nombre es sólo una etiqueta; lo que se verifica es la firma del `Hola`
//  contra la clave declarada, y la huella se contrasta una vez fuera de banda
//  (TOFU), igual que en `ayni`/`agora`.
//
//  Override `UYA_SEMILLA=<64 hex>` fija la semilla (tests/demos deterministas);
//  `UYA_IDENTIDAD=<ruta>` cambia el archivo. Sin nada, se persiste en
//  `$XDG_DATA_HOME/uya/identidad` (o `~/.local/share/uya/identidad`).
// =============================================================================

use std::path::PathBuf;

use agora_core::{verify_signature, Keypair};
use rand::RngCore;

use uya_core::{id_desde_clave, mensaje_identidad, ParticipanteId};

/// La identidad firmante local: su par Ed25519 y la semilla que lo engendra
/// (retenida sólo en memoria, para alimentar también el keypair de transporte).
pub struct Identidad {
    seed: [u8; 32],
    kp: Keypair,
}

impl Identidad {
    /// Resuelve la identidad local: `UYA_SEMILLA` si está, si no el archivo
    /// persistido, si no una semilla nueva del CSPRNG que se guarda para la
    /// próxima. Nunca falla "duro": si el disco no coopera, sigue con una
    /// semilla en memoria (la sesión anda; sólo no persiste).
    pub fn cargar() -> Self {
        if let Some(seed) = semilla_de_entorno() {
            return Self::desde_semilla(seed);
        }
        let ruta = ruta_identidad();
        if let Some(seed) = leer_semilla(&ruta) {
            return Self::desde_semilla(seed);
        }
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);
        if let Err(e) = guardar_semilla(&ruta, &seed) {
            eprintln!("uya: no pude persistir la identidad en {ruta:?}: {e} (sesión efímera)");
        }
        Self::desde_semilla(seed)
    }

    /// Deriva una identidad de una semilla de 32 bytes (determinista).
    pub fn desde_semilla(seed: [u8; 32]) -> Self {
        Self {
            seed,
            kp: Keypair::from_seed(seed),
        }
    }

    /// La semilla raíz (para el keypair libp2p del transporte).
    pub fn semilla(&self) -> [u8; 32] {
        self.seed
    }

    /// La clave pública Ed25519 (32 bytes).
    pub fn clave(&self) -> [u8; 32] {
        self.kp.public_key()
    }

    /// El id del participante = huella BLAKE3 de la clave pública.
    pub fn id(&self) -> ParticipanteId {
        id_desde_clave(&self.clave())
    }

    /// Firma la atestación de identidad `(id, nombre)` para el `Hola`.
    pub fn firmar_presentacion(&self, nombre: &str) -> Vec<u8> {
        self.kp.sign(&mensaje_identidad(&self.id(), nombre)).to_vec()
    }
}

/// Verifica el `Hola` de un par: que el `id` declarado sea efectivamente la
/// huella de la `clave`, y que la `firma` valide contra esa clave sobre
/// `mensaje_identidad(id, nombre)`. Sólo entonces el par cuenta como verificado.
pub fn verificar_presentacion(
    id: &ParticipanteId,
    nombre: &str,
    clave: &[u8; 32],
    firma: &[u8],
) -> bool {
    if id_desde_clave(clave) != *id {
        return false;
    }
    let Ok(firma) = <[u8; 64]>::try_from(firma) else {
        return false;
    };
    verify_signature(clave, &mensaje_identidad(id, nombre), &firma).is_ok()
}

/// La semilla fijada por `UYA_SEMILLA` (64 hex), si está presente y es válida.
fn semilla_de_entorno() -> Option<[u8; 32]> {
    let hex = std::env::var("UYA_SEMILLA").ok()?;
    desde_hex(hex.trim())
}

/// Dónde se persiste la semilla: `UYA_IDENTIDAD`, o `$XDG_DATA_HOME/uya/...`, o
/// `~/.local/share/uya/identidad`.
fn ruta_identidad() -> PathBuf {
    if let Ok(p) = std::env::var("UYA_IDENTIDAD") {
        return PathBuf::from(p);
    }
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .ok()
        .or_else(|| std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("uya").join("identidad")
}

/// Lee 32 bytes crudos de semilla del archivo, si existe y mide bien.
fn leer_semilla(ruta: &PathBuf) -> Option<[u8; 32]> {
    let bytes = std::fs::read(ruta).ok()?;
    <[u8; 32]>::try_from(bytes.as_slice()).ok()
}

/// Escribe la semilla cruda con permisos restrictivos (0600 en Unix).
fn guardar_semilla(ruta: &PathBuf, seed: &[u8; 32]) -> std::io::Result<()> {
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(ruta, seed)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(ruta, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Parsea 64 caracteres hex a 32 bytes.
fn desde_hex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_estable_para_una_semilla() {
        let a = Identidad::desde_semilla([3u8; 32]);
        let b = Identidad::desde_semilla([3u8; 32]);
        assert_eq!(a.id(), b.id());
        assert_eq!(a.clave(), b.clave());
        // Semillas distintas → ids distintos.
        assert_ne!(a.id(), Identidad::desde_semilla([4u8; 32]).id());
    }

    #[test]
    fn presentacion_firmada_verifica() {
        let yo = Identidad::desde_semilla([10u8; 32]);
        let firma = yo.firmar_presentacion("Alicia");
        assert!(verificar_presentacion(&yo.id(), "Alicia", &yo.clave(), &firma));
    }

    #[test]
    fn presentacion_con_nombre_cambiado_falla() {
        let yo = Identidad::desde_semilla([11u8; 32]);
        let firma = yo.firmar_presentacion("Alicia");
        // Misma clave/id pero nombre distinto: la firma no cubre "Mallory".
        assert!(!verificar_presentacion(&yo.id(), "Mallory", &yo.clave(), &firma));
    }

    #[test]
    fn suplantacion_con_otra_clave_falla() {
        // Mallory firma con SU clave un Hola que reclama el id de Alicia.
        let alicia = Identidad::desde_semilla([12u8; 32]);
        let mallory = Identidad::desde_semilla([13u8; 32]);
        let firma = mallory.firmar_presentacion("Alicia");
        // id de Alicia + clave de Mallory: id_desde_clave(mallory) != id_alicia.
        assert!(!verificar_presentacion(&alicia.id(), "Alicia", &mallory.clave(), &firma));
    }

    #[test]
    fn hex_roundtrip() {
        let seed = [0xabu8; 32];
        let hex: String = seed.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(desde_hex(&hex), Some(seed));
        assert_eq!(desde_hex("corto"), None);
    }
}
