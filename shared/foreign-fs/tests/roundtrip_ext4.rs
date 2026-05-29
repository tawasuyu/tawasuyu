// =============================================================================
//  foreign-fs :: prueba de absorción ext4/ext2 == importación host
// -----------------------------------------------------------------------------
//  Espejo de la prueba FAT, ahora sobre el FS nativo de Linux. Un mismo árbol
//  absorbido por el lector ext (sobre la imagen cruda) debe colapsar al MISMO
//  hash raíz que importarlo del disco con la lógica de grafo del host. A
//  diferencia de FAT, ext4 SÍ preserva bit de ejecución y enlaces simbólicos,
//  así que el árbol de prueba los incluye y el oráculo los reproduce.
//
//  Una salvedad real del medio: `mke2fs` crea un directorio `lost+found` vacío
//  en la raíz que el árbol fuente no tiene. El oráculo lo inyecta explícitamente
//  (un subárbol vacío) — es contabilidad honesta del único artefacto que el FS
//  añade, no un parche al lector.
//
//  Ejercita: ext4 (árbol de extents) y ext2 (bloques indirectos directo/simple/
//  doble, forzados con bloques de 1 KiB y un archivo de 600 KiB), symlinks
//  rápidos, exec bit, archivo vacío y troceado > 256 KiB.
//
//  Requiere `mke2fs` (e2fsprogs). Si falta, la prueba se salta limpiamente.
// =============================================================================

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use foreign_fs::{absorber, ext4::LectorExt4, Emisor, EmisorMemoria, TAMANO_TROZO};

fn which(bin: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {bin}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn patron(n: usize, semilla: u8) -> Vec<u8> {
    (0..n).map(|i| ((i + semilla as usize) % 251) as u8).collect()
}

/// Árbol de prueba con exec bit y symlink (ext los preserva).
fn sembrar_arbol(raiz: &Path) {
    fs::create_dir_all(raiz).unwrap();
    fs::write(raiz.join("hola_mundo.txt"), b"hola, grafo\n").unwrap();
    fs::write(raiz.join("READ.ME"), b"corto\n").unwrap();
    fs::write(raiz.join("vacio.dat"), b"").unwrap();
    fs::write(raiz.join("datos_grandes.bin"), patron(600_000, 3)).unwrap(); // troceado + indirecto doble

    let sh = raiz.join("script.sh");
    fs::write(&sh, b"#!/bin/sh\necho hola\n").unwrap();
    fs::set_permissions(&sh, fs::Permissions::from_mode(0o755)).unwrap();

    std::os::unix::fs::symlink("hola_mundo.txt", raiz.join("enlace")).unwrap();

    let sub = raiz.join("subcarpeta");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("anidado.txt"), b"profundo\n").unwrap();
    fs::write(sub.join("otro.bin"), patron(300_000, 11)).unwrap();
}

// --- Oráculo: espejo COMPLETO de agora-cli importar (con exec + symlink) -----

fn oraculo_dir(dir: &Path, emisor: &mut EmisorMemoria, es_raiz: bool) -> format::Hash {
    let mut entradas: Vec<format::EntradaArbol> = Vec::new();
    for ent in fs::read_dir(dir).unwrap() {
        let ent = ent.unwrap();
        let ruta = ent.path();
        let nombre = ent.file_name().to_string_lossy().into_owned();
        let ft = ent.file_type().unwrap(); // no sigue symlinks
        let (modo, hash) = if ft.is_symlink() {
            let destino = fs::read_link(&ruta).unwrap();
            let bytes = destino.to_string_lossy().into_owned().into_bytes();
            (
                format::ModoEntrada::Symlink,
                emisor.emitir(&format::objeto_blob(bytes)).unwrap(),
            )
        } else if ft.is_dir() {
            (
                format::ModoEntrada::Directorio,
                oraculo_dir(&ruta, emisor, false),
            )
        } else {
            let bytes = fs::read(&ruta).unwrap();
            let ejecutable = fs::metadata(&ruta).unwrap().permissions().mode() & 0o111 != 0;
            let modo = if ejecutable {
                format::ModoEntrada::Ejecutable
            } else {
                format::ModoEntrada::Archivo
            };
            (modo, oraculo_archivo(bytes, emisor))
        };
        entradas.push(format::EntradaArbol { nombre, modo, hash });
    }
    // `mke2fs` añade un lost+found vacío en la raíz; el FS lo tiene, el árbol
    // fuente no. Se contabiliza como un subárbol vacío.
    if es_raiz {
        let vacio = emisor.emitir(&format::objeto_arbol(Vec::new()).unwrap()).unwrap();
        entradas.push(format::EntradaArbol {
            nombre: "lost+found".into(),
            modo: format::ModoEntrada::Directorio,
            hash: vacio,
        });
    }
    let objeto = format::objeto_arbol(entradas).unwrap();
    emisor.emitir(&objeto).unwrap()
}

fn oraculo_archivo(bytes: Vec<u8>, emisor: &mut EmisorMemoria) -> format::Hash {
    if bytes.len() <= TAMANO_TROZO {
        return emisor.emitir(&format::objeto_blob(bytes)).unwrap();
    }
    let mut trozos = Vec::new();
    for trozo in bytes.chunks(TAMANO_TROZO) {
        trozos.push(emisor.emitir(&format::objeto_blob(trozo.to_vec())).unwrap());
    }
    emisor.emitir(&format::objeto_blob_indice(trozos)).unwrap()
}

// --- Forja de la imagen ext con mke2fs -d ------------------------------------

fn forjar_imagen(src: &Path, tipo: &str, img: &Path) {
    let f = fs::File::create(img).unwrap();
    f.set_len(64 * 1024 * 1024).unwrap();
    drop(f);
    let salida = Command::new("mke2fs")
        .args(["-q", "-F", "-b", "1024", "-t", tipo, "-d"])
        .arg(src)
        .arg(img)
        .output()
        .unwrap();
    assert!(
        salida.status.success(),
        "mke2fs -t {tipo} falló: {}",
        String::from_utf8_lossy(&salida.stderr)
    );
}

fn comparar(tipo: &str) {
    if !which("mke2fs") {
        eprintln!("SKIP: falta mke2fs");
        return;
    }
    let tmp = tmpdir(&format!("ff-{tipo}"));
    let src = tmp.join("src");
    sembrar_arbol(&src);
    let img = tmp.join(format!("{tipo}.img"));
    forjar_imagen(&src, tipo, &img);

    let bytes = fs::read(&img).unwrap();
    let lector = LectorExt4::nuevo(bytes.as_slice())
        .unwrap_or_else(|e| panic!("{tipo}: superbloque no parseó: {e:?}"));
    let mut emisor_fs = EmisorMemoria::nuevo();
    let raiz_fs = absorber(&lector, &mut emisor_fs)
        .unwrap_or_else(|e| panic!("{tipo}: absorción falló: {e:?}"));

    let mut emisor_oraculo = EmisorMemoria::nuevo();
    let raiz_oraculo = oraculo_dir(&src, &mut emisor_oraculo, true);

    assert_eq!(
        raiz_fs, raiz_oraculo,
        "{tipo}: el hash raíz de la absorción difiere del importado del disco"
    );
    assert_eq!(
        emisor_fs.len(),
        emisor_oraculo.len(),
        "{tipo}: distinto número de objetos únicos"
    );

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn ext4_coincide_con_importar() {
    comparar("ext4");
}

#[test]
fn ext2_coincide_con_importar() {
    comparar("ext2");
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
