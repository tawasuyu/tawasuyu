// =============================================================================
//  nahual-source-core :: DispositivosSource — navegación de un medio real
// -----------------------------------------------------------------------------
//  Forja imágenes de FS pobladas (FAT con mcopy, ext4 con mke2fs -d), las trata
//  como "dispositivos" vía `DispositivosSource::con_dispositivos`, y verifica
//  que la fuente las navega y lee SIN montar — la mirada soberana de manejar
//  particiones.
//
//  Requiere `mkfs.fat` + `mcopy` (dosfstools + mtools) y/o `mke2fs`. Skip
//  limpio si faltan, para no romper en una máquina pelada.
// =============================================================================

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nahual_source_core::{DispositivoInfo, DispositivosSource, Source};

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

/// Imagen FAT suelta (sin tabla de particiones) con un archivo y un subdir.
fn imagen_fat(tmp: &Path) -> PathBuf {
    let img = tmp.join("fat.img");
    fs::File::create(&img).unwrap().set_len(4 * 1024 * 1024).unwrap();
    let s = Command::new("mkfs.fat").arg(&img).output().unwrap();
    assert!(s.status.success(), "mkfs.fat: {}", String::from_utf8_lossy(&s.stderr));

    // Un archivo en la raíz y un subdir con otro archivo.
    let hola = tmp.join("hola.txt");
    fs::write(&hola, b"hola dispositivo\n").unwrap();
    let s = Command::new("mcopy")
        .arg("-i").arg(&img).arg(&hola).arg("::hola.txt")
        .output().unwrap();
    assert!(s.status.success(), "mcopy archivo: {}", String::from_utf8_lossy(&s.stderr));

    let s = Command::new("mmd").arg("-i").arg(&img).arg("::sub").output().unwrap();
    assert!(s.status.success(), "mmd: {}", String::from_utf8_lossy(&s.stderr));
    let dentro = tmp.join("dentro.bin");
    fs::write(&dentro, vec![7u8; 1000]).unwrap();
    let s = Command::new("mcopy")
        .arg("-i").arg(&img).arg(&dentro).arg("::sub/dentro.bin")
        .output().unwrap();
    assert!(s.status.success(), "mcopy subdir: {}", String::from_utf8_lossy(&s.stderr));

    img
}

fn info_de(img: &Path, nombre: &str) -> DispositivoInfo {
    let tam = fs::metadata(img).unwrap().len();
    DispositivoInfo {
        ruta: img.to_path_buf(),
        nombre: nombre.into(),
        tam: Some(tam),
        removible: true,
        modelo: None,
    }
}

#[test]
fn navega_un_fat_suelto_hasta_leer_un_archivo() {
    if !which("mkfs.fat") || !which("mcopy") || !which("mmd") {
        eprintln!("SKIP: faltan mkfs.fat/mcopy/mmd");
        return;
    }
    let tmp = tmpdir("disp-fat");
    let img = imagen_fat(&tmp);
    let src = DispositivosSource::con_dispositivos(vec![info_de(&img, "usb0")]);

    // Raíz → un dispositivo.
    let root = src.root();
    assert_eq!(root.id, "@dispositivos");
    let devs = src.children(&root.id).unwrap();
    assert_eq!(devs.len(), 1, "un dispositivo");
    assert!(devs[0].is_container);

    // Dispositivo → una partición FAT (FS suelto).
    let parts = src.children(&devs[0].id).unwrap();
    assert_eq!(parts.len(), 1, "una partición");
    assert!(parts[0].name.contains("FAT"), "etiqueta FAT, fue: {}", parts[0].name);
    assert!(parts[0].is_container);

    // Partición → raíz del FS: el archivo, el subdir.
    let raiz_fs = src.children(&parts[0].id).unwrap();
    let hola = raiz_fs.iter().find(|n| n.name == "hola.txt").expect("hola.txt");
    assert!(!hola.is_container);
    assert_eq!(hola.size, Some(17)); // "hola dispositivo\n"
    let sub = raiz_fs.iter().find(|n| n.name == "sub").expect("subdir sub");
    assert!(sub.is_container);

    // Leer el archivo de la raíz.
    let bytes = src.read(&hola.id).unwrap();
    assert_eq!(bytes, b"hola dispositivo\n");

    // Descender al subdir y leer su archivo.
    let dentro = src.children(&sub.id).unwrap();
    let bin = dentro.iter().find(|n| n.name == "dentro.bin").expect("dentro.bin");
    assert_eq!(src.read(&bin.id).unwrap(), vec![7u8; 1000]);

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn navega_un_ext4_suelto() {
    if !which("mke2fs") {
        eprintln!("SKIP: falta mke2fs");
        return;
    }
    let tmp = tmpdir("disp-ext");
    // Poblar un dir y volcarlo a una imagen ext4 con `mke2fs -d`.
    let pobla = tmp.join("pobla");
    fs::create_dir_all(pobla.join("dir")).unwrap();
    fs::write(pobla.join("raiz.txt"), b"contenido ext\n").unwrap();
    fs::write(pobla.join("dir/anidado.txt"), b"anidado\n").unwrap();

    let img = tmp.join("ext.img");
    fs::File::create(&img).unwrap().set_len(16 * 1024 * 1024).unwrap();
    let s = Command::new("mke2fs")
        .args(["-q", "-F", "-b", "1024", "-t", "ext4", "-d"])
        .arg(&pobla).arg(&img).output().unwrap();
    assert!(s.status.success(), "mke2fs: {}", String::from_utf8_lossy(&s.stderr));

    let src = DispositivosSource::con_dispositivos(vec![info_de(&img, "disco0")]);
    let devs = src.children(&src.root().id).unwrap();
    let parts = src.children(&devs[0].id).unwrap();
    assert_eq!(parts.len(), 1);
    assert!(parts[0].name.contains("ext"), "etiqueta ext, fue: {}", parts[0].name);

    let raiz_fs = src.children(&parts[0].id).unwrap();
    let raiz_txt = raiz_fs.iter().find(|n| n.name == "raiz.txt").expect("raiz.txt");
    assert_eq!(src.read(&raiz_txt.id).unwrap(), b"contenido ext\n");

    let dir = raiz_fs.iter().find(|n| n.name == "dir").expect("dir");
    let dentro = src.children(&dir.id).unwrap();
    let anid = dentro.iter().find(|n| n.name == "anidado.txt").expect("anidado.txt");
    assert_eq!(src.read(&anid.id).unwrap(), b"anidado\n");

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn read_preview_acota_la_cabeza() {
    if !which("mkfs.fat") || !which("mcopy") || !which("mmd") {
        eprintln!("SKIP: faltan mkfs.fat/mcopy/mmd");
        return;
    }
    const TOPE: usize = 16 * 1024 * 1024; // = TOPE_PREVIEW (privado en el crate)
    let tmp = tmpdir("disp-preview");
    let img = tmp.join("fat.img");
    fs::File::create(&img).unwrap().set_len(48 * 1024 * 1024).unwrap();
    assert!(Command::new("mkfs.fat").arg(&img).output().unwrap().status.success());

    // Archivo de 17 MiB (> tope) y uno chico (< tope).
    let grande = tmp.join("grande.bin");
    fs::write(&grande, vec![0xABu8; 17 * 1024 * 1024]).unwrap();
    assert!(Command::new("mcopy").arg("-i").arg(&img).arg(&grande).arg("::grande.bin").output().unwrap().status.success());
    let chico = tmp.join("chico.txt");
    fs::write(&chico, b"corto\n").unwrap();
    assert!(Command::new("mcopy").arg("-i").arg(&img).arg(&chico).arg("::chico.txt").output().unwrap().status.success());

    let src = DispositivosSource::con_dispositivos(vec![info_de(&img, "usb0")]);
    let parts = src.children(&src.children(&src.root().id).unwrap()[0].id).unwrap();
    let files = src.children(&parts[0].id).unwrap();

    let g = files.iter().find(|n| n.name == "grande.bin").unwrap();
    // Preview acota a TOPE; read completo trae los 17 MiB.
    assert_eq!(src.read_preview(&g.id).unwrap().len(), TOPE);
    assert_eq!(src.read(&g.id).unwrap().len(), 17 * 1024 * 1024);

    let c = files.iter().find(|n| n.name == "chico.txt").unwrap();
    // Un archivo chico: preview == read (no se trunca de más).
    assert_eq!(src.read_preview(&c.id).unwrap(), b"corto\n");

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn id_basura_es_error_no_panic() {
    let src = DispositivosSource::con_dispositivos(vec![]);
    assert!(src.children(&"no-existe".to_string()).is_err());
    assert!(src.read(&"basura".to_string()).is_err());
    // La raíz sin dispositivos lista vacío, sin reventar.
    assert!(src.children(&src.root().id).unwrap().is_empty());
}

#[test]
fn enumeracion_real_no_panic() {
    // No asevera contenido (depende de la máquina), sólo que no revienta.
    let src = DispositivosSource::nueva();
    let _ = src.children(&src.root().id).unwrap();
}
