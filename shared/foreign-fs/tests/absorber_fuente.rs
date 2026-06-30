// =============================================================================
//  foreign-fs :: absorber sobre Fuente == absorber sobre &[u8]
// -----------------------------------------------------------------------------
//  Verifica que tragar un dispositivo por la vía PEREZOSA (`*_fuente`, leyendo
//  por offset como lo haría un `/dev/sdX`) produce EXACTAMENTE el mismo grafo
//  —mismo hash raíz, mismos objetos— que la vía residente `&[u8]`. Es la prueba
//  de que la lectura perezosa no altera la identidad de contenido.
//
//  Requiere `mke2fs`. Skip si falta.
// =============================================================================

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use foreign_fs::particion::{absorber_dispositivo, absorber_dispositivo_fuente};
use foreign_fs::{EmisorMemoria, Fuente, FsError};

fn which(bin: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {bin}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
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

/// Imagen ext4 suelta poblada con un árbol pequeño.
fn imagen_ext(tmp: &Path) -> Vec<u8> {
    let src = tmp.join("src");
    fs::create_dir_all(src.join("dir")).unwrap();
    fs::write(src.join("raiz.txt"), b"contenido\n").unwrap();
    fs::write(src.join("dir/anidado.bin"), vec![3u8; 4096]).unwrap();

    let img = tmp.join("ext.img");
    fs::File::create(&img).unwrap().set_len(16 * 1024 * 1024).unwrap();
    let s = Command::new("mke2fs")
        .args(["-q", "-F", "-b", "1024", "-t", "ext4", "-d"])
        .arg(&src).arg(&img).output().unwrap();
    assert!(s.status.success(), "mke2fs: {}", String::from_utf8_lossy(&s.stderr));
    fs::read(&img).unwrap()
}

/// `Fuente` que sirve por offset desde un `Vec` propio (como un syscall),
/// distinta de `&[u8]` para probar el camino genérico de verdad.
struct FuentePropia(Vec<u8>);
impl Fuente for FuentePropia {
    fn tamano(&self) -> u64 {
        self.0.len() as u64
    }
    fn leer_en(&self, offset: u64, buf: &mut [u8]) -> Result<(), FsError> {
        let ini = offset as usize;
        let fin = ini + buf.len();
        let src = self.0.get(ini..fin).ok_or(FsError::Corrupto("fuera del medio"))?;
        buf.copy_from_slice(src);
        Ok(())
    }
}

#[test]
fn fuente_y_slice_absorben_el_mismo_grafo() {
    if !which("mke2fs") {
        eprintln!("SKIP: falta mke2fs");
        return;
    }
    let tmp = tmpdir("ff-abs-eq");
    let bytes = imagen_ext(&tmp);

    let mut e_slice = EmisorMemoria::nuevo();
    let raiz_slice = absorber_dispositivo(bytes.as_slice(), &mut e_slice).unwrap();

    let mut e_fuente = EmisorMemoria::nuevo();
    let raiz_fuente =
        absorber_dispositivo_fuente(&FuentePropia(bytes.clone()), &mut e_fuente).unwrap();

    assert_eq!(raiz_slice, raiz_fuente, "misma identidad de contenido");
    assert_eq!(e_slice.len(), e_fuente.len(), "mismo conjunto de objetos");

    let _ = fs::remove_dir_all(&tmp);
}

#[cfg(feature = "std")]
#[test]
fn fuente_archivo_absorbe_igual_que_el_slice() {
    if !which("mke2fs") {
        eprintln!("SKIP: falta mke2fs");
        return;
    }
    let tmp = tmpdir("ff-abs-file");
    let bytes = imagen_ext(&tmp);
    let img = tmp.join("ext.img"); // imagen_ext ya la escribió

    // Vía &[u8] residente.
    let mut e_slice = EmisorMemoria::nuevo();
    let raiz_slice = absorber_dispositivo(bytes.as_slice(), &mut e_slice).unwrap();

    // Vía FuenteArchivo sobre el archivo en disco (perezosa, como un /dev/sdX).
    let fa = foreign_fs::FuenteArchivo::abrir(&img).unwrap();
    let mut e_file = EmisorMemoria::nuevo();
    let raiz_file = absorber_dispositivo_fuente(&fa, &mut e_file).unwrap();

    assert_eq!(raiz_slice, raiz_file, "leer del archivo da el mismo grafo");
    assert_eq!(e_slice.len(), e_file.len());

    let _ = fs::remove_dir_all(&tmp);
}
