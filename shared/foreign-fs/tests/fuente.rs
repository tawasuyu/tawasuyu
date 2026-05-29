// =============================================================================
//  foreign-fs :: prueba del trait Fuente — equivalencia + memoria acotada
// -----------------------------------------------------------------------------
//  El refactor a `Fuente` persigue dos cosas, y aquí se verifican ambas:
//
//   1. EQUIVALENCIA: absorber a través de una `Fuente` ARBITRARIA (no el `&[u8]`
//      del host, sino una que sirve lecturas por offset como lo haría un syscall
//      in-cage) produce el MISMO hash raíz que el `&[u8]`. Es decir, los
//      lectores no asumen un slice contiguo residente.
//
//   2. MEMORIA ACOTADA: el absorbedor NUNCA pide el archivo entero de una. Una
//      `Fuente` espía registra la lectura más grande que se le pidió; al
//      absorber un archivo de 2.5 MiB, esa lectura máxima queda en el orden del
//      bloque (≤ 4 KiB) — prueba de que el contenido se recorre por streaming,
//      no materializando el archivo. Es lo que vuelve viable correr bajo el
//      techo de 4 MiB in-cage.
//
//  Requiere `mke2fs`. Skip si falta.
// =============================================================================

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

use foreign_fs::{absorber, ext4::LectorExt4, EmisorMemoria, Fuente, FsError};

fn which(bin: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {bin}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Una `Fuente` que sirve por offset desde un `Vec` propio (como lo haría un
/// syscall in-cage) y espía la lectura individual más grande que se le pidió.
struct FuenteEspia {
    datos: Vec<u8>,
    max_lectura: Rc<Cell<usize>>,
}

impl FuenteEspia {
    fn nueva(datos: Vec<u8>) -> Self {
        Self { datos, max_lectura: Rc::new(Cell::new(0)) }
    }
}

impl Fuente for FuenteEspia {
    fn tamano(&self) -> u64 {
        self.datos.len() as u64
    }
    fn leer_en(&self, offset: u64, buf: &mut [u8]) -> Result<(), FsError> {
        if buf.len() > self.max_lectura.get() {
            self.max_lectura.set(buf.len());
        }
        let ini = offset as usize;
        let fin = ini
            .checked_add(buf.len())
            .ok_or(FsError::Corrupto("offset desbordó"))?;
        let src = self
            .datos
            .get(ini..fin)
            .ok_or(FsError::Corrupto("lectura fuera del medio"))?;
        buf.copy_from_slice(src);
        Ok(())
    }
}

fn imagen_ext_con_archivo_grande(tmp: &Path) -> Vec<u8> {
    let src = tmp.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("chico.txt"), b"hola\n").unwrap();
    // 2.5 MiB → muy por encima de un trozo (256 KiB) y de un bloque (1-4 KiB).
    let grande: Vec<u8> = (0..2_500_000u32).map(|i| (i % 251) as u8).collect();
    fs::write(src.join("grande.bin"), &grande).unwrap();

    let img = tmp.join("ext.img");
    fs::File::create(&img).unwrap().set_len(32 * 1024 * 1024).unwrap();
    let s = Command::new("mke2fs")
        .args(["-q", "-F", "-b", "1024", "-t", "ext4", "-d"])
        .arg(&src).arg(&img).output().unwrap();
    assert!(s.status.success(), "mke2fs: {}", String::from_utf8_lossy(&s.stderr));
    fs::read(&img).unwrap()
}

#[test]
fn fuente_arbitraria_da_el_mismo_hash() {
    if !which("mke2fs") {
        eprintln!("SKIP: falta mke2fs");
        return;
    }
    let tmp = tmpdir("ff-fuente-eq");
    let bytes = imagen_ext_con_archivo_grande(&tmp);

    // Vía &[u8] (host).
    let mut e_slice = EmisorMemoria::nuevo();
    let raiz_slice = absorber(&LectorExt4::nuevo(bytes.as_slice()).unwrap(), &mut e_slice).unwrap();

    // Vía Fuente propia (síncrona, sirve por offset como un syscall).
    let espia = FuenteEspia::nueva(bytes.clone());
    let mut e_fuente = EmisorMemoria::nuevo();
    let raiz_fuente = absorber(&LectorExt4::nuevo(espia).unwrap(), &mut e_fuente).unwrap();

    assert_eq!(raiz_slice, raiz_fuente, "la Fuente arbitraria debe dar el mismo grafo");
    assert_eq!(e_slice.len(), e_fuente.len());

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn absorbedor_lee_acotado_no_el_archivo_entero() {
    if !which("mke2fs") {
        eprintln!("SKIP: falta mke2fs");
        return;
    }
    let tmp = tmpdir("ff-fuente-mem");
    let bytes = imagen_ext_con_archivo_grande(&tmp);

    let espia = FuenteEspia::nueva(bytes);
    // El lector retiene la Fuente; quedamos con un handle al contador ANTES de
    // moverla, para consultarlo tras absorber.
    let max_handle = espia.max_lectura.clone();
    let lector = LectorExt4::nuevo(espia).unwrap();
    let mut e = EmisorMemoria::nuevo();
    absorber(&lector, &mut e).unwrap();

    // La lectura individual más grande pedida a la Fuente. Con bloques de 1 KiB
    // y todo (superbloque, inodos, bloques de datos, dir) leído por bloque, el
    // máximo queda muy por debajo del archivo de 2.5 MiB y por debajo de un
    // trozo de 256 KiB → el archivo NO se lee entero.
    let max = max_handle.get();
    assert!(
        max <= 4096,
        "lectura máxima {max} B: el absorbedor no debería pedir más de un bloque a la vez"
    );

    let _ = fs::remove_dir_all(&tmp);
}

fn tmpdir(prefijo: &str) -> PathBuf {
    let base = std::env::var("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let dir = base.join(format!("{prefijo}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}
