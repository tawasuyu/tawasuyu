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
use std::path::Path;
use std::sync::Mutex;

use crate::{Fuente, FsError};

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
