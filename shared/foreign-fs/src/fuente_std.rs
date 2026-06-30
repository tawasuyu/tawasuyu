// =============================================================================
//  foreign-fs :: fuente_std — `Fuente` sobre un archivo/dispositivo real (host)
// -----------------------------------------------------------------------------
//  La abstracción `Fuente` es no_std (sirve por offset y nada más). Para un
//  dispositivo de bloques REAL del host —`/dev/sdX`, una imagen `.img`— hace
//  falta un respaldo con `std::fs::File`: leer por `seek` + `read_exact`, bajo
//  demanda, sin cargar el medio entero a RAM. Eso es `FuenteArchivo`.
//
//  Vive tras la feature `std` para no contaminar el núcleo no_std (kernel /
//  in-cage). Consumidores host (nahual, agora-cli) la prenden y comparten esta
//  única implementación en vez de duplicar el `seek+read_exact`.
// =============================================================================

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::{Emisor, Fuente, FsError};

/// Una [`Fuente`] respaldada por un archivo o dispositivo de bloques. El `Mutex`
/// resuelve dos cosas: `Fuente::leer_en` toma `&self` pero `File` necesita
/// `&mut` para posicionar, y vuelve el tipo `Sync` (lo exigen los consumidores
/// que la comparten entre hilos).
pub struct FuenteArchivo {
    f: Mutex<File>,
    tam: u64,
}

impl FuenteArchivo {
    /// Abre `ruta` en sólo-lectura. El tamaño se obtiene con un `seek` al final:
    /// en un dispositivo de bloques `metadata().len()` es 0, pero el `seek`
    /// devuelve el tamaño real tanto para `/dev/sdX` como para un archivo.
    pub fn abrir(ruta: impl AsRef<Path>) -> std::io::Result<Self> {
        let mut f = File::open(ruta)?;
        let tam = f.seek(SeekFrom::End(0))?;
        Ok(Self { f: Mutex::new(f), tam })
    }
}

impl Fuente for FuenteArchivo {
    fn tamano(&self) -> u64 {
        self.tam
    }
    fn leer_en(&self, offset: u64, buf: &mut [u8]) -> Result<(), FsError> {
        let mut f = self
            .f
            .lock()
            .map_err(|_| FsError::Corrupto("lock del dispositivo envenenado"))?;
        f.seek(SeekFrom::Start(offset))
            .map_err(|_| FsError::Corrupto("seek del dispositivo falló"))?;
        f.read_exact(buf)
            .map_err(|_| FsError::Corrupto("lectura corta del dispositivo"))?;
        Ok(())
    }
}

/// Codifica un hash de 32 bytes a 64 chars hex en minúscula —el nombre con que
/// el bundle direccionado por contenido nombra cada objeto (`<hash>.obj`)—.
pub fn hex32(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Un [`Emisor`] que persiste cada objeto del grafo como `<hash>.obj` en un
/// directorio bundle —el mismo formato servible por `servir_release`—. Es la
/// contraparte sink de [`FuenteArchivo`] (host-side): donde aquélla LEE el
/// dispositivo, ésta ESCRIBE el grafo absorbido. Hogar único del `<hash>.obj`,
/// hoy duplicado en `agora-cli`.
pub struct EmisorBundle {
    dir: PathBuf,
    error_io: Option<std::io::Error>,
}

impl EmisorBundle {
    pub fn nuevo(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into(), error_io: None }
    }

    /// El error de I/O real de la primera emisión que falló (si hubo), para
    /// reportarlo con detalle en vez del genérico [`FsError::EmisionFallida`].
    pub fn tomar_error_io(&mut self) -> Option<std::io::Error> {
        self.error_io.take()
    }
}

impl Emisor for EmisorBundle {
    fn emitir(&mut self, objeto: &format::Objeto) -> Result<format::Hash, FsError> {
        let payload = objeto.serializar().map_err(FsError::Format)?;
        let hash = format::hash(&payload);
        let ruta = self.dir.join(format!("{}.obj", hex32(&hash)));
        if let Err(e) = std::fs::write(&ruta, &payload) {
            self.error_io.get_or_insert(e);
            return Err(FsError::EmisionFallida);
        }
        Ok(hash)
    }
}
