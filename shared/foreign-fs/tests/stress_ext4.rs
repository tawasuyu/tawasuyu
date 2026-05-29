// =============================================================================
//  foreign-fs :: stress ext4 — multi-grupo de bloques + htree + datos variados
// -----------------------------------------------------------------------------
//  Las pruebas de round-trip básicas usan árboles diminutos en UN solo block
//  group con directorios de UN bloque. Un ext4 REAL no es así. Esta prueba
//  fuerza, sobre una imagen de 128 MiB (verificado: 23 block groups):
//    * miles de inodos repartidos en VARIOS block groups (inodes_per_group=2048
//      < 2500 archivos) → ejercita la indexación de descriptores de grupo;
//    * un directorio GRANDE de muchos bloques → el contenido del dir se mapea
//      por extents y se parsea linealmente a través de todos sus bloques;
//    * el salto de entradas `inode==0` → presente en cada bloque de dir como el
//      tail de `metadata_csum` (feature activa por defecto en mke2fs);
//    * archivo multi-MiB → extents sobre cientos de bloques;
//    * nombres unicode/con espacios, hard links, exec y symlinks.
//  (mke2fs -d arma directorios LINEALES grandes, no htree-indexados —el índice
//  htree lo construye el kernel al insertar en un FS montado, fuera de alcance
//  sin loopback—; el lector parsea ambos por igual, pero esta prueba cubre el
//  caso lineal multi-bloque, no el índice htree en sí.)
//
//  La verificación es la de siempre: el hash raíz de la absorción ext debe
//  igualar al de importar el MISMO árbol del disco. Si la matemática de grupos
//  o el recorrido de bloques estuvieran mal, faltarían/corromperían archivos y
//  el hash divergiría. Es autovalidante.
//
//  Requiere `mke2fs`. Skip si falta. Más pesada que el resto (miles de
//  archivos), pero igual corre en ~1 s.
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
    (0..n).map(|i| ((i.wrapping_mul(31) + semilla as usize) % 251) as u8).collect()
}

/// Árbol grande y variado.
fn sembrar(raiz: &Path) {
    fs::create_dir_all(raiz).unwrap();

    // Un directorio con MUCHAS entradas → dir lineal de muchos bloques.
    let muchos = raiz.join("muchos");
    fs::create_dir_all(&muchos).unwrap();
    for i in 0..2500 {
        // Contenido que varía por i → no todo deduplica a un blob.
        fs::write(muchos.join(format!("archivo_{i:05}.dat")), patron(8 + i % 64, (i % 251) as u8)).unwrap();
    }

    // Anidamiento profundo.
    let mut d = raiz.join("hondo");
    for k in 0..10 {
        d = d.join(format!("nivel{k}"));
    }
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("fondo.txt"), b"el fondo\n").unwrap();

    // Archivo multi-MiB → extents sobre muchos bloques.
    fs::write(raiz.join("pesado.bin"), patron(2_500_000, 9)).unwrap();
    // Justo en el límite de trozo y uno más.
    fs::write(raiz.join("limite.bin"), patron(TAMANO_TROZO, 1)).unwrap();

    // Nombres unicode y con espacios.
    fs::write(raiz.join("ñandú león.txt"), "acentos\n".as_bytes()).unwrap();
    fs::write(raiz.join("文件.bin"), patron(1234, 5)).unwrap();

    // Exec, symlink y hard link.
    let sh = raiz.join("correr.sh");
    fs::write(&sh, b"#!/bin/sh\necho ok\n").unwrap();
    fs::set_permissions(&sh, fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink("correr.sh", raiz.join("atajo")).unwrap();
    fs::write(raiz.join("original.txt"), b"compartido\n").unwrap();
    fs::hard_link(raiz.join("original.txt"), raiz.join("gemelo.txt")).unwrap();
}

// --- Oráculo: importación host (con exec + symlink + lost+found en raíz) ------

fn oraculo_dir(dir: &Path, emisor: &mut EmisorMemoria, es_raiz: bool) -> format::Hash {
    let mut entradas: Vec<format::EntradaArbol> = Vec::new();
    for ent in fs::read_dir(dir).unwrap() {
        let ent = ent.unwrap();
        let ruta = ent.path();
        let nombre = ent.file_name().to_string_lossy().into_owned();
        let ft = ent.file_type().unwrap();
        let (modo, hash) = if ft.is_symlink() {
            let destino = fs::read_link(&ruta).unwrap();
            let bytes = destino.to_string_lossy().into_owned().into_bytes();
            (format::ModoEntrada::Symlink, emisor.emitir(&format::objeto_blob(bytes)).unwrap())
        } else if ft.is_dir() {
            (format::ModoEntrada::Directorio, oraculo_dir(&ruta, emisor, false))
        } else {
            let bytes = fs::read(&ruta).unwrap();
            let ejecutable = fs::metadata(&ruta).unwrap().permissions().mode() & 0o111 != 0;
            let modo = if ejecutable { format::ModoEntrada::Ejecutable } else { format::ModoEntrada::Archivo };
            (modo, oraculo_archivo(bytes, emisor))
        };
        entradas.push(format::EntradaArbol { nombre, modo, hash });
    }
    if es_raiz {
        let vacio = emisor.emitir(&format::objeto_arbol(Vec::new()).unwrap()).unwrap();
        entradas.push(format::EntradaArbol {
            nombre: "lost+found".into(),
            modo: format::ModoEntrada::Directorio,
            hash: vacio,
        });
    }
    emisor.emitir(&format::objeto_arbol(entradas).unwrap()).unwrap()
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

#[test]
fn ext4_arbol_grande_coincide_con_importar() {
    if !which("mke2fs") {
        eprintln!("SKIP: falta mke2fs");
        return;
    }
    let tmp = tmpdir("ff-stress");
    let src = tmp.join("src");
    sembrar(&src);

    // 128 MiB con bloques de 1 KiB ⇒ muchos block groups (≈8 MiB/grupo) y, con
    // el ratio de inodos por defecto, los ~2500 inodos cruzan varios grupos.
    let img = tmp.join("ext.img");
    fs::File::create(&img).unwrap().set_len(128 * 1024 * 1024).unwrap();
    let s = Command::new("mke2fs")
        .args(["-q", "-F", "-b", "1024", "-t", "ext4", "-d"])
        .arg(&src).arg(&img).output().unwrap();
    assert!(s.status.success(), "mke2fs: {}", String::from_utf8_lossy(&s.stderr));

    let bytes = fs::read(&img).unwrap();
    let lector = LectorExt4::nuevo(bytes.as_slice()).expect("superbloque");
    let mut e_fs = EmisorMemoria::nuevo();
    let raiz_fs = absorber(&lector, &mut e_fs).expect("absorción");

    let mut e_or = EmisorMemoria::nuevo();
    let raiz_or = oraculo_dir(&src, &mut e_or, true);

    assert_eq!(raiz_fs, raiz_or, "hash raíz difiere — multi-grupo o dir multi-bloque mal leído");
    assert_eq!(e_fs.len(), e_or.len(), "distinto número de objetos únicos");

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
