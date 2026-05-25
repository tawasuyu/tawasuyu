//! `wawa-explorer-core` — lector host-side de imágenes Wawa.
//!
//! Abre un archivo `.img` con el formato de disco de Wawa (definido en
//! `shared/format`), lee el SuperBloque del sector 0, hace replay del log
//! de objetos y reconstruye el grafo direccionado por contenido en memoria.
//! Expone una API navegable: manifiesto, raíz de userspace, objeto por
//! hash, listado de hijos.
//!
//! Es estrictamente lectura. La autoridad sobre el disco es del kernel y
//! del boot — este crate no escribe NADA. Pensado para herramientas de
//! debug, viewers, snapshot inspectors.
//!
//! Sin red. El cliente AoE (gossip de objetos a peers Wawa) vive aparte
//! en `wawa-explorer-aoe` para no arrastrar deps de raw sockets a quien
//! solo quiere mirar un archivo.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use format::{
    longitud_registro, sectores_registro, Hash, Manifiesto, Objeto, SuperBloque, MAGIA,
    MAX_OBJETO, TAM_SECTOR, VERSION_SUPERBLOQUE,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("magia inválida en el sector 0: {hallada:02x?} (esperaba {esperada:02x?})")]
    MagiaInvalida { hallada: [u8; 8], esperada: [u8; 8] },
    #[error("versión de superbloque desconocida: {hallada} (esta build entiende {esperada})")]
    VersionDesconocida { hallada: u32, esperada: u32 },
    #[error("superbloque corrupto: {0}")]
    SuperbloqueCorrupto(&'static str),
    #[error("objeto corrupto en sector {sector}: {motivo}")]
    ObjetoCorrupto { sector: u64, motivo: &'static str },
    #[error("manifiesto inválido: {0}")]
    ManifiestoInvalido(&'static str),
    #[error("objeto {0} referenciado por el superbloque no existe en el log")]
    ReferenciaColgante(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Imagen Wawa cargada en memoria: superbloque + grafo entero.
///
/// Carga eager: leemos todo el log de una vez. Las imágenes Wawa son
/// chicas (típicamente decenas de MB) — pagar el costo al abrir y luego
/// servir lookups O(1) por hash es lo correcto. Si en el futuro alguien
/// quiere imágenes gigantes, el patrón es agregar un `LazyDisco` que
/// indexa offsets sin desserializar.
#[derive(Debug)]
pub struct Disco {
    superbloque: SuperBloque,
    objetos: HashMap<Hash, Objeto>,
    /// Total de bytes del archivo leído — útil para mostrar en el header.
    bytes_imagen: u64,
}

impl Disco {
    /// Abre y carga una imagen Wawa desde disco.
    pub fn abrir(ruta: &Path) -> Result<Self> {
        let mut f = File::open(ruta)?;
        let bytes_imagen = f.metadata()?.len();

        let superbloque = leer_superbloque(&mut f)?;
        let objetos = replay_log(&mut f, &superbloque)?;

        // Validar consistencia: los anchors del superbloque deben existir
        // en el grafo cargado, si no son None.
        if let Some(h) = superbloque.raiz {
            if !objetos.contains_key(&h) {
                return Err(Error::ReferenciaColgante(short_hex(&h)));
            }
        }
        if let Some(h) = superbloque.manifiesto {
            if !objetos.contains_key(&h) {
                return Err(Error::ReferenciaColgante(short_hex(&h)));
            }
        }

        Ok(Self { superbloque, objetos, bytes_imagen })
    }

    pub fn superbloque(&self) -> &SuperBloque {
        &self.superbloque
    }

    pub fn bytes_imagen(&self) -> u64 {
        self.bytes_imagen
    }

    pub fn cantidad_objetos(&self) -> usize {
        self.objetos.len()
    }

    /// Iterador sobre todos los hashes del grafo.
    pub fn hashes(&self) -> impl Iterator<Item = &Hash> {
        self.objetos.keys()
    }

    /// Objeto por su hash, si está cargado.
    pub fn objeto(&self, hash: &Hash) -> Option<&Objeto> {
        self.objetos.get(hash)
    }

    /// Hashes de los hijos directos de un objeto, o `None` si no existe.
    pub fn hijos(&self, hash: &Hash) -> Option<&[Hash]> {
        self.objetos.get(hash).map(|o| o.hijos.as_slice())
    }

    /// El manifiesto deserializado, si el superbloque lo apunta y la
    /// carga es válida.
    pub fn manifiesto(&self) -> Result<Option<Manifiesto>> {
        let Some(hash) = self.superbloque.manifiesto else {
            return Ok(None);
        };
        let objeto = self
            .objetos
            .get(&hash)
            .ok_or_else(|| Error::ReferenciaColgante(short_hex(&hash)))?;
        let m = Manifiesto::deserializar(&objeto.datos).map_err(Error::ManifiestoInvalido)?;
        Ok(Some(m))
    }
}

fn leer_superbloque(f: &mut File) -> Result<SuperBloque> {
    f.seek(SeekFrom::Start(0))?;
    let mut sector = vec![0u8; TAM_SECTOR];
    f.read_exact(&mut sector)?;
    let sb = SuperBloque::deserializar(&sector)
        .map_err(Error::SuperbloqueCorrupto)?;
    if sb.magia != MAGIA {
        return Err(Error::MagiaInvalida { hallada: sb.magia, esperada: MAGIA });
    }
    if sb.version != VERSION_SUPERBLOQUE {
        return Err(Error::VersionDesconocida {
            hallada: sb.version,
            esperada: VERSION_SUPERBLOQUE,
        });
    }
    Ok(sb)
}

fn replay_log(f: &mut File, sb: &SuperBloque) -> Result<HashMap<Hash, Objeto>> {
    let mut objetos = HashMap::new();
    let mut sector_actual: u64 = 1;

    while sector_actual < sb.cursor {
        let offset = sector_actual * TAM_SECTOR as u64;
        f.seek(SeekFrom::Start(offset))?;

        let mut cabecera = [0u8; 4];
        if f.read(&mut cabecera)? < 4 {
            break;
        }
        let Some(longitud) = longitud_registro(&cabecera) else {
            // Fin del log (cabecera a cero) o longitud disparatada. Si el
            // cursor declara más sectores, los ignoramos: el kernel deja
            // padding sin escribir entre log writes en algunos casos.
            break;
        };
        if longitud > MAX_OBJETO {
            return Err(Error::ObjetoCorrupto {
                sector: sector_actual,
                motivo: "longitud declarada excede MAX_OBJETO",
            });
        }

        // Leer el payload completo. componer_registro asegura que cabe en
        // sectores_registro(longitud) sectores; volvemos a leer desde el
        // inicio del registro (offset + 4) hasta longitud bytes.
        f.seek(SeekFrom::Start(offset + 4))?;
        let mut payload = vec![0u8; longitud];
        f.read_exact(&mut payload)?;

        let objeto = Objeto::deserializar(&payload).map_err(|_| Error::ObjetoCorrupto {
            sector: sector_actual,
            motivo: "deserialización postcard falló",
        })?;
        let bytes_canonicos = objeto.serializar().map_err(|_| Error::ObjetoCorrupto {
            sector: sector_actual,
            motivo: "re-serialización para hash falló",
        })?;
        let id = format::hash(&bytes_canonicos);
        objetos.insert(id, objeto);

        sector_actual += sectores_registro(longitud);
    }

    Ok(objetos)
}

/// Formato corto de un hash para mensajes de error y logs: primeros 6 bytes en hex.
pub fn short_hex(h: &Hash) -> String {
    h[..6].iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use format::{componer_registro, EntradaApp, Manifiesto, MAGIA, VERSION_MANIFIESTO};
    use std::io::Write;

    /// Construye un disco sintético en un archivo temporal: SuperBloque +
    /// dos objetos en el log, uno de los cuales es el manifiesto.
    fn disco_sintetico() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("test.img");
        let mut f = File::create(&ruta).unwrap();

        // Objeto 1: bytecode dummy.
        let bytecode = Objeto { datos: vec![0xAA; 100], hijos: vec![] };
        let bytecode_payload = bytecode.serializar().unwrap();
        let bytecode_hash = format::hash(&bytecode_payload);

        // Objeto 2: manifiesto que apunta al bytecode.
        let manifiesto = Manifiesto {
            version: VERSION_MANIFIESTO,
            apps: vec![EntradaApp {
                nombre: "test-app".into(),
                bytecode: bytecode_hash,
                region_x: 0,
                region_y: 0,
                region_ancho: 640,
                region_alto: 480,
                techo_memoria: 4 * 1024 * 1024,
                estado: None,
            }],
        };
        let manifest_obj = Objeto {
            datos: manifiesto.serializar().unwrap(),
            hijos: vec![bytecode_hash],
        };
        let manifest_payload = manifest_obj.serializar().unwrap();
        let manifest_hash = format::hash(&manifest_payload);

        // Trazado en disco: sector 0 = SuperBloque, sector 1+ = log.
        let registro_bytecode = componer_registro(&bytecode_payload);
        let registro_manifest = componer_registro(&manifest_payload);
        let sectores_bytecode = registro_bytecode.len() / TAM_SECTOR;
        let sectores_manifest = registro_manifest.len() / TAM_SECTOR;
        let cursor_final = 1 + sectores_bytecode as u64 + sectores_manifest as u64;

        let sb = SuperBloque {
            magia: MAGIA,
            version: VERSION_SUPERBLOQUE,
            cursor: cursor_final,
            raiz: None,
            manifiesto: Some(manifest_hash),
        };
        let sb_bytes = sb.serializar().unwrap();
        let mut sb_sector = vec![0u8; TAM_SECTOR];
        sb_sector[..sb_bytes.len()].copy_from_slice(&sb_bytes);

        f.write_all(&sb_sector).unwrap();
        f.write_all(&registro_bytecode).unwrap();
        f.write_all(&registro_manifest).unwrap();
        f.sync_all().unwrap();

        (dir, ruta)
    }

    #[test]
    fn abrir_disco_sintetico_carga_grafo_completo() {
        let (_dir, ruta) = disco_sintetico();
        let disco = Disco::abrir(&ruta).unwrap();
        assert_eq!(disco.cantidad_objetos(), 2);
        assert!(disco.superbloque().manifiesto.is_some());
        assert!(disco.superbloque().raiz.is_none());
    }

    #[test]
    fn manifiesto_navega_a_la_app() {
        let (_dir, ruta) = disco_sintetico();
        let disco = Disco::abrir(&ruta).unwrap();
        let manifest = disco.manifiesto().unwrap().unwrap();
        assert_eq!(manifest.apps.len(), 1);
        assert_eq!(manifest.apps[0].nombre, "test-app");
        // El bytecode referenciado existe en el grafo.
        assert!(disco.objeto(&manifest.apps[0].bytecode).is_some());
    }

    #[test]
    fn magia_invalida_es_error_legible() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("falso.img");
        let mut f = File::create(&ruta).unwrap();
        // Sector 0 con magia "OTRABASURA" + version 2 + cursor + raiz/manifest None.
        let sb = SuperBloque {
            magia: *b"OTRABASU",
            version: VERSION_SUPERBLOQUE,
            cursor: 1,
            raiz: None,
            manifiesto: None,
        };
        let sb_bytes = sb.serializar().unwrap();
        let mut sector = vec![0u8; TAM_SECTOR];
        sector[..sb_bytes.len()].copy_from_slice(&sb_bytes);
        f.write_all(&sector).unwrap();
        f.sync_all().unwrap();

        let err = Disco::abrir(&ruta).unwrap_err();
        assert!(matches!(err, Error::MagiaInvalida { .. }), "fue {err:?}");
    }

    #[test]
    fn referencia_colgante_falla() {
        // SuperBloque que apunta a un manifiesto inexistente.
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("colgante.img");
        let mut f = File::create(&ruta).unwrap();
        let sb = SuperBloque {
            magia: MAGIA,
            version: VERSION_SUPERBLOQUE,
            cursor: 1,
            raiz: None,
            manifiesto: Some([0x42; 32]),
        };
        let sb_bytes = sb.serializar().unwrap();
        let mut sector = vec![0u8; TAM_SECTOR];
        sector[..sb_bytes.len()].copy_from_slice(&sb_bytes);
        f.write_all(&sector).unwrap();
        f.sync_all().unwrap();

        let err = Disco::abrir(&ruta).unwrap_err();
        assert!(matches!(err, Error::ReferenciaColgante(_)), "fue {err:?}");
    }
}
