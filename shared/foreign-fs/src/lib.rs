// =============================================================================
//  foreign-fs :: absorción de sistemas de archivos ajenos al grafo
// -----------------------------------------------------------------------------
//  Dos piezas independientes:
//
//    1. La ABSTRACCIÓN de un FS de sólo-lectura (`LectorFs`) + el ABSORBEDOR
//       (`absorber`) que recorre ese FS y emite objetos del grafo nativo. El
//       absorbedor es agnóstico del FS de origen y reproduce, bit a bit, la
//       construcción de grafo del host (`agora-cli wawa importar`): mismo
//       troceado de 256 KiB, mismo `objeto_arbol` ordenado por nombre, mismo
//       criterio blob-plano vs índice. Si dos árboles tienen idéntico
//       contenido, colapsan al MISMO hash raíz, vengan de donde vengan.
//
//    2. Lectores `LectorFs` concretos: FAT12/16/32 (`fat`) y ext2/3/4 (`ext4`),
//       cada uno sobre un slice de bytes crudo —exactamente lo que wawa ve de un
//       dispositivo de bloques sin montar—. FAT cubre el USB/partición EFI;
//       ext4 cubre la partición vieja de Linux del usuario.
//
//  El destino de los objetos lo decide un `Emisor`: en el host escribe
//  `<hash>.obj`; in-cage llamaría a `sys_object_put`. El absorbedor sólo habla
//  el idioma `format::Objeto`.
// =============================================================================
#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

pub mod ext4;
pub mod fat;

/// Tamaño de trozo para archivos grandes. IDÉNTICO al host
/// (`agora-cli::TAMANO_TROZO`): 256 KiB << `MAX_OBJETO` (1 MiB). Cambiarlo aquí
/// rompería la identidad de hash con los bundles importados en el host.
pub const TAMANO_TROZO: usize = 256 * 1024;

/// Error de absorción. Variantes deliberadamente pocas: el origen (FAT) aporta
/// su propio detalle como `&'static str` para no acoplar un enum gigante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsError {
    /// El medio no parsea como el FS esperado (magia/BPB inválido, etc.).
    MedioInvalido(&'static str),
    /// Una referencia (cluster/inode) apunta fuera del medio o forma un ciclo.
    Corrupto(&'static str),
    /// El `Emisor` falló al persistir un objeto.
    EmisionFallida,
    /// El format del grafo rechazó la construcción (p.ej. árbol inválido).
    Format(&'static str),
}

/// Clase de una entrada de directorio en el FS de origen, ya normalizada a los
/// modos del grafo. FAT no tiene ni bit de ejecución ni enlaces simbólicos, así
/// que su lector sólo produce `Archivo`/`Directorio`; ext4 sí los preserva y su
/// lector emite las cuatro clases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clase {
    /// Archivo regular. `ejecutable` espeja el bit `x` de Unix (siempre `false`
    /// en FAT).
    Archivo { ejecutable: bool },
    /// Subdirectorio: hay que recorrerlo.
    Directorio,
    /// Enlace simbólico: el contenido es la ruta destino en UTF-8.
    Symlink,
}

/// Una entrada listada de un directorio: su nombre, su clase y la manija opaca
/// con que el `LectorFs` la vuelve a abrir (leer archivo, listar subdir, leer
/// destino de symlink).
pub struct EntradaDir<M> {
    pub nombre: String,
    pub clase: Clase,
    pub manija: M,
}

/// Un sistema de archivos de sólo-lectura, recorrible nodo a nodo. La `Manija`
/// es opaca al absorbedor: sólo el lector sabe interpretarla (en FAT lleva el
/// cluster inicial y, para archivos, el tamaño).
pub trait LectorFs {
    /// Identificador opaco de un nodo del FS.
    type Manija: Clone;

    /// La manija del directorio raíz.
    fn raiz(&self) -> Self::Manija;

    /// Lista las entradas de un directorio. NO incluye `.`/`..`. El orden es
    /// irrelevante: el absorbedor reordena por nombre vía `objeto_arbol`.
    fn listar(&self, dir: &Self::Manija) -> Result<Vec<EntradaDir<Self::Manija>>, FsError>;

    /// Lee el contenido completo de un archivo regular.
    fn leer_archivo(&self, archivo: &Self::Manija) -> Result<Vec<u8>, FsError>;

    /// Lee el destino (ruta en UTF-8) de un enlace simbólico. FAT nunca lo
    /// invoca; un lector que no soporte symlinks puede devolver
    /// `Err(MedioInvalido)`.
    fn destino_symlink(&self, _enlace: &Self::Manija) -> Result<String, FsError> {
        Err(FsError::MedioInvalido("este FS no soporta enlaces simbólicos"))
    }
}

/// Sumidero de objetos del grafo. Espeja `agora-cli::emitir_objeto`: serializa,
/// hashea, persiste, devuelve el hash. El absorbedor no sabe (ni le importa) si
/// el objeto va a `<hash>.obj`, a `sys_object_put` o a un mapa en memoria.
pub trait Emisor {
    /// Persiste un objeto y devuelve su hash (sobre la forma serializada). DEBE
    /// ser idempotente: emitir dos veces el mismo objeto devuelve el mismo
    /// hash sin efecto observable extra.
    fn emitir(&mut self, objeto: &format::Objeto) -> Result<format::Hash, FsError>;
}

/// Absorbe un FS entero al grafo, de abajo hacia arriba, y devuelve el hash del
/// árbol RAÍZ —el hash único que representa todo el contenido del medio—.
pub fn absorber<L: LectorFs, E: Emisor>(fs: &L, emisor: &mut E) -> Result<format::Hash, FsError> {
    absorber_dir(fs, &fs.raiz(), emisor)
}

/// Absorbe un directorio recursivamente. Espejo EXACTO de
/// `agora-cli::importar_dir`: cada entrada se convierte en una `EntradaArbol`
/// (`nombre` + `modo` + `hash`) y el directorio entero en un `objeto_arbol`
/// (que ORDENA por nombre, garantizando determinismo de hash).
fn absorber_dir<L: LectorFs, E: Emisor>(
    fs: &L,
    dir: &L::Manija,
    emisor: &mut E,
) -> Result<format::Hash, FsError> {
    let mut entradas: Vec<format::EntradaArbol> = Vec::new();
    for ent in fs.listar(dir)? {
        let (modo, hash) = match ent.clase {
            Clase::Symlink => {
                let destino = fs.destino_symlink(&ent.manija)?;
                let hash = emisor.emitir(&format::objeto_blob(destino.into_bytes()))?;
                (format::ModoEntrada::Symlink, hash)
            }
            Clase::Directorio => {
                let hash = absorber_dir(fs, &ent.manija, emisor)?;
                (format::ModoEntrada::Directorio, hash)
            }
            Clase::Archivo { ejecutable } => {
                let bytes = fs.leer_archivo(&ent.manija)?;
                let hash = absorber_archivo(bytes, emisor)?;
                let modo = if ejecutable {
                    format::ModoEntrada::Ejecutable
                } else {
                    format::ModoEntrada::Archivo
                };
                (modo, hash)
            }
        };
        entradas.push(format::EntradaArbol {
            nombre: ent.nombre,
            modo,
            hash,
        });
    }
    let objeto = format::objeto_arbol(entradas).map_err(FsError::Format)?;
    emisor.emitir(&objeto)
}

/// Absorbe el contenido de un archivo: blob plano si cabe en un trozo, o un
/// índice de trozos si es grande. Espejo EXACTO de `agora-cli::importar_archivo`.
fn absorber_archivo<E: Emisor>(bytes: Vec<u8>, emisor: &mut E) -> Result<format::Hash, FsError> {
    if bytes.len() <= TAMANO_TROZO {
        return emisor.emitir(&format::objeto_blob(bytes));
    }
    let mut trozos: Vec<format::Hash> = Vec::new();
    for trozo in bytes.chunks(TAMANO_TROZO) {
        trozos.push(emisor.emitir(&format::objeto_blob(trozo.to_vec()))?);
    }
    emisor.emitir(&format::objeto_blob_indice(trozos))
}

/// Un `Emisor` que acumula los objetos en memoria, indexados por hash. Útil
/// para pruebas y para un consumidor que quiera el grafo completo antes de
/// volcarlo (p.ej. un bundle servible por Akasha). Dedup gratis: hashes
/// repetidos sobreescriben la misma entrada.
#[derive(Default)]
pub struct EmisorMemoria {
    objetos: alloc::collections::BTreeMap<format::Hash, Vec<u8>>,
}

impl EmisorMemoria {
    pub fn nuevo() -> Self {
        Self {
            objetos: alloc::collections::BTreeMap::new(),
        }
    }

    /// Número de objetos ÚNICOS absorbidos (= archivos `.obj` que produciría el
    /// bundle host).
    pub fn len(&self) -> usize {
        self.objetos.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objetos.is_empty()
    }

    /// La carga útil serializada de un objeto por su hash, si está presente.
    pub fn obtener(&self, hash: &format::Hash) -> Option<&Vec<u8>> {
        self.objetos.get(hash)
    }
}

impl Emisor for EmisorMemoria {
    fn emitir(&mut self, objeto: &format::Objeto) -> Result<format::Hash, FsError> {
        let payload = objeto.serializar().map_err(FsError::Format)?;
        let hash = format::hash(&payload);
        self.objetos.insert(hash, payload);
        Ok(hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    /// FS sintético en memoria: un árbol de nodos sin depender de herramientas
    /// FAT. Sirve para fijar la lógica del absorbedor (orden, troceado, dedup)
    /// en cualquier máquina.
    enum NodoMem {
        Archivo(Vec<u8>, bool), // (contenido, ejecutable)
        Dir(Vec<(String, NodoMem)>),
        Symlink(String),
    }

    struct FsMem {
        raiz: NodoMem,
    }

    // La manija es la ruta de índices desde la raíz hasta el nodo.
    impl FsMem {
        fn nodo(&self, ruta: &[usize]) -> &NodoMem {
            let mut n = &self.raiz;
            for &i in ruta {
                match n {
                    NodoMem::Dir(hijos) => n = &hijos[i].1,
                    _ => panic!("ruta inválida"),
                }
            }
            n
        }
    }

    impl LectorFs for FsMem {
        type Manija = Vec<usize>;
        fn raiz(&self) -> Vec<usize> {
            Vec::new()
        }
        fn listar(&self, dir: &Vec<usize>) -> Result<Vec<EntradaDir<Vec<usize>>>, FsError> {
            match self.nodo(dir) {
                NodoMem::Dir(hijos) => Ok(hijos
                    .iter()
                    .enumerate()
                    .map(|(i, (nombre, nodo))| {
                        let mut manija = dir.clone();
                        manija.push(i);
                        let clase = match nodo {
                            NodoMem::Archivo(_, ej) => Clase::Archivo { ejecutable: *ej },
                            NodoMem::Dir(_) => Clase::Directorio,
                            NodoMem::Symlink(_) => Clase::Symlink,
                        };
                        EntradaDir {
                            nombre: nombre.clone(),
                            clase,
                            manija,
                        }
                    })
                    .collect()),
                _ => Err(FsError::Corrupto("listar sobre no-directorio")),
            }
        }
        fn leer_archivo(&self, m: &Vec<usize>) -> Result<Vec<u8>, FsError> {
            match self.nodo(m) {
                NodoMem::Archivo(b, _) => Ok(b.clone()),
                _ => Err(FsError::Corrupto("leer no-archivo")),
            }
        }
        fn destino_symlink(&self, m: &Vec<usize>) -> Result<String, FsError> {
            match self.nodo(m) {
                NodoMem::Symlink(d) => Ok(d.clone()),
                _ => Err(FsError::Corrupto("symlink sobre no-symlink")),
            }
        }
    }

    fn arbol_demo() -> FsMem {
        FsMem {
            raiz: NodoMem::Dir(vec![
                ("a.txt".into(), NodoMem::Archivo(b"hola".to_vec(), false)),
                ("b.txt".into(), NodoMem::Archivo(b"hola".to_vec(), false)), // dup contenido
                (
                    "sub".into(),
                    NodoMem::Dir(vec![(
                        "c.bin".into(),
                        NodoMem::Archivo(b"hola".to_vec(), false),
                    )]),
                ),
                ("enlace".into(), NodoMem::Symlink("a.txt".into())),
            ]),
        }
    }

    #[test]
    fn absorber_es_determinista() {
        let fs = arbol_demo();
        let mut e1 = EmisorMemoria::nuevo();
        let mut e2 = EmisorMemoria::nuevo();
        let r1 = absorber(&fs, &mut e1).unwrap();
        let r2 = absorber(&fs, &mut e2).unwrap();
        assert_eq!(r1, r2, "misma entrada → mismo hash raíz");
    }

    #[test]
    fn dedup_por_contenido() {
        // 3 archivos con idéntico contenido ("hola") comparten UN solo blob;
        // el symlink añade un blob distinto (su destino "a.txt"). Objetos
        // únicos = 1 blob "hola" + 1 blob "a.txt" + 1 subárbol + 1 árbol raíz = 4.
        let fs = arbol_demo();
        let mut e = EmisorMemoria::nuevo();
        absorber(&fs, &mut e).unwrap();
        assert_eq!(e.len(), 4, "blob deduplicado + blob symlink + 2 árboles");
    }

    #[test]
    fn troceado_en_el_limite() {
        // Exactamente TAMANO_TROZO → blob plano (1 objeto). Un byte más →
        // índice + 2 trozos (3 objetos).
        let mut justo = EmisorMemoria::nuevo();
        let h_justo = absorber_archivo(vec![0u8; TAMANO_TROZO], &mut justo).unwrap();
        assert_eq!(justo.len(), 1, "en el límite es blob plano");
        // El blob plano debe ser el propio objeto (sin hijos).
        assert!(justo.obtener(&h_justo).is_some());

        let mut grande = EmisorMemoria::nuevo();
        absorber_archivo(vec![0u8; TAMANO_TROZO + 1], &mut grande).unwrap();
        assert_eq!(grande.len(), 3, "pasado el límite: índice + 2 trozos");
    }
}
