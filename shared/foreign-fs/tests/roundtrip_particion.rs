// =============================================================================
//  foreign-fs :: prueba de la capa de particiones (GPT / MBR / FS suelto)
// -----------------------------------------------------------------------------
//  Forja un disco PARTICIONADO sin loopback: escribe la tabla con `sfdisk` y
//  luego inyecta imágenes de FS ya pobladas (FAT con mcopy, ext4 con mke2fs -d)
//  en los offsets de cada partición. Verifica que la capa:
//    * localiza las particiones en el offset y tamaño correctos,
//    * autodetecta el FS de cada una (FAT vs ext),
//    * `absorber_particion` produce el MISMO hash que absorber la imagen FS
//      suelta (la sub-slice de la partición ES la imagen),
//    * `absorber_dispositivo` arma el árbol top `particionN` determinista.
//  Y que un FS suelto (sin tabla) se trata como una única partición.
//
//  Requiere `sfdisk` + `mkfs.fat`/`mcopy` + `mke2fs`. Skip si faltan.
// =============================================================================

use std::fs;
use std::io::Write;
use std::os::unix::fs::{FileExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use foreign_fs::particion::{
    absorber_dispositivo, absorber_particion, detectar_fs, tabla_particiones, Esquema,
    SistemaArchivos,
};
use foreign_fs::{absorber, ext4::LectorExt4, fat::LectorFat, Emisor, EmisorMemoria};

fn which(bin: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {bin}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn herramientas() -> bool {
    which("sfdisk") && which("mkfs.fat") && which("mcopy") && which("mke2fs")
}

// --- Imágenes de FS pobladas (standalone) ------------------------------------

fn sembrar_fat(raiz: &Path) {
    fs::create_dir_all(raiz.join("sub")).unwrap();
    fs::write(raiz.join("hola_mundo.txt"), b"contenido fat\n").unwrap();
    fs::write(raiz.join("sub").join("anidado.txt"), b"profundo\n").unwrap();
}

fn sembrar_ext(raiz: &Path) {
    fs::create_dir_all(raiz.join("sub")).unwrap();
    fs::write(raiz.join("a.txt"), b"contenido ext\n").unwrap();
    let sh = raiz.join("run.sh");
    fs::write(&sh, b"#!/bin/sh\necho hi\n").unwrap();
    fs::set_permissions(&sh, fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink("a.txt", raiz.join("enlace")).unwrap();
    fs::write(raiz.join("sub").join("x.txt"), b"anidado\n").unwrap();
}

/// Imagen FAT16 de `mib` MiB, poblada con los hijos de `src`.
fn imagen_fat(tmp: &Path, src: &Path, mib: u64) -> Vec<u8> {
    let img = tmp.join("fat.img");
    fs::File::create(&img).unwrap().set_len(mib * 1024 * 1024).unwrap();
    // Sin forzar -F: mkfs.fat elige el ancho de FAT según el tamaño; el lector
    // maneja 12/16/32 por igual.
    let s = Command::new("mkfs.fat").arg(&img).output().unwrap();
    assert!(s.status.success(), "mkfs.fat: {}", String::from_utf8_lossy(&s.stderr));
    for ent in fs::read_dir(src).unwrap() {
        let ruta = ent.unwrap().path();
        let s = Command::new("mcopy")
            .env("MTOOLS_SKIP_CHECK", "1")
            .arg("-s").arg("-i").arg(&img).arg(&ruta).arg("::")
            .output().unwrap();
        assert!(s.status.success(), "mcopy: {}", String::from_utf8_lossy(&s.stderr));
    }
    fs::read(&img).unwrap()
}

/// Imagen ext4 de `mib` MiB, poblada con `src` vía `mke2fs -d`.
fn imagen_ext(tmp: &Path, src: &Path, mib: u64) -> Vec<u8> {
    let img = tmp.join("ext.img");
    fs::File::create(&img).unwrap().set_len(mib * 1024 * 1024).unwrap();
    let s = Command::new("mke2fs")
        .args(["-q", "-F", "-b", "1024", "-t", "ext4", "-d"])
        .arg(src).arg(&img).output().unwrap();
    assert!(s.status.success(), "mke2fs: {}", String::from_utf8_lossy(&s.stderr));
    fs::read(&img).unwrap()
}

// --- Forja del disco particionado --------------------------------------------

fn align_up(x: u64, a: u64) -> u64 {
    (x + a - 1) / a * a
}

fn sfdisk(img: &Path, script: &str) {
    let mut hijo = Command::new("sfdisk")
        .arg(img)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    hijo.stdin.as_mut().unwrap().write_all(script.as_bytes()).unwrap();
    let s = hijo.wait_with_output().unwrap();
    assert!(s.status.success(), "sfdisk: {}", String::from_utf8_lossy(&s.stderr));
}

fn escribir_en(img: &Path, offset: u64, bytes: &[u8]) {
    let f = fs::OpenOptions::new().write(true).open(img).unwrap();
    f.write_all_at(bytes, offset).unwrap();
}

// -----------------------------------------------------------------------------

#[test]
fn gpt_fat_y_ext_coinciden() {
    if !herramientas() {
        eprintln!("SKIP: faltan sfdisk/mkfs.fat/mcopy/mke2fs");
        return;
    }
    let tmp = tmpdir("ff-gpt");
    let src_fat = tmp.join("src-fat");
    let src_ext = tmp.join("src-ext");
    sembrar_fat(&src_fat);
    sembrar_ext(&src_ext);
    let fat = imagen_fat(&tmp, &src_fat, 16);
    let ext = imagen_ext(&tmp, &src_ext, 16);
    let fat_sec = (fat.len() as u64) / 512;
    let ext_sec = (ext.len() as u64) / 512;

    let p1_start = 2048u64;
    let p2_start = align_up(p1_start + fat_sec, 2048);
    let total = (p2_start + ext_sec + 2048) * 512;

    let disco = tmp.join("disco.img");
    fs::File::create(&disco).unwrap().set_len(total).unwrap();
    sfdisk(
        &disco,
        &format!(
            "label: gpt\nstart={p1_start}, size={fat_sec}, type=uefi\nstart={p2_start}, size={ext_sec}, type=linux\n"
        ),
    );
    escribir_en(&disco, p1_start * 512, &fat);
    escribir_en(&disco, p2_start * 512, &ext);

    let datos = fs::read(&disco).unwrap();
    let parts = tabla_particiones(&datos).unwrap();
    assert_eq!(parts.len(), 2, "GPT con dos particiones");
    assert_eq!(parts[0].esquema, Esquema::Gpt);
    assert_eq!(parts[0].inicio, p1_start * 512);
    assert_eq!(parts[0].tam, fat.len() as u64);
    assert_eq!(parts[1].inicio, p2_start * 512);
    assert_eq!(parts[1].tam, ext.len() as u64);

    // Autodetección de FS por partición.
    let s1 = &datos[parts[0].inicio as usize..(parts[0].inicio + parts[0].tam) as usize];
    let s2 = &datos[parts[1].inicio as usize..(parts[1].inicio + parts[1].tam) as usize];
    assert_eq!(detectar_fs(s1), SistemaArchivos::Fat);
    assert_eq!(detectar_fs(s2), SistemaArchivos::Ext);

    // absorber_particion == absorber la imagen FS suelta.
    let r_fat_std = {
        let mut e = EmisorMemoria::nuevo();
        absorber(&LectorFat::nuevo(fat.as_slice()).unwrap(), &mut e).unwrap()
    };
    let r_ext_std = {
        let mut e = EmisorMemoria::nuevo();
        absorber(&LectorExt4::nuevo(ext.as_slice()).unwrap(), &mut e).unwrap()
    };
    let mut e1 = EmisorMemoria::nuevo();
    assert_eq!(absorber_particion(&datos, &parts[0], &mut e1).unwrap(), r_fat_std);
    let mut e2 = EmisorMemoria::nuevo();
    assert_eq!(absorber_particion(&datos, &parts[1], &mut e2).unwrap(), r_ext_std);

    // absorber_dispositivo == árbol top {particion1->fat, particion2->ext}.
    let mut e_dev = EmisorMemoria::nuevo();
    let raiz_dev = absorber_dispositivo(&datos, &mut e_dev).unwrap();
    let mut e_man = EmisorMemoria::nuevo();
    let r_fat = absorber(&LectorFat::nuevo(fat.as_slice()).unwrap(), &mut e_man).unwrap();
    let r_ext = absorber(&LectorExt4::nuevo(ext.as_slice()).unwrap(), &mut e_man).unwrap();
    let top = format::objeto_arbol(vec![
        format::EntradaArbol { nombre: "particion1".into(), modo: format::ModoEntrada::Directorio, hash: r_fat },
        format::EntradaArbol { nombre: "particion2".into(), modo: format::ModoEntrada::Directorio, hash: r_ext },
    ]).unwrap();
    let raiz_man = e_man.emitir(&top).unwrap();
    assert_eq!(raiz_dev, raiz_man, "árbol de dispositivo determinista");

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn mbr_una_particion_ext() {
    if !herramientas() {
        eprintln!("SKIP: faltan herramientas");
        return;
    }
    let tmp = tmpdir("ff-mbr");
    let src = tmp.join("src");
    sembrar_ext(&src);
    let ext = imagen_ext(&tmp, &src, 16);
    let ext_sec = (ext.len() as u64) / 512;
    let start = 2048u64;
    let total = (start + ext_sec + 2048) * 512;

    let disco = tmp.join("disco.img");
    fs::File::create(&disco).unwrap().set_len(total).unwrap();
    sfdisk(&disco, &format!("label: dos\nstart={start}, size={ext_sec}, type=83\n"));
    escribir_en(&disco, start * 512, &ext);

    let datos = fs::read(&disco).unwrap();
    let parts = tabla_particiones(&datos).unwrap();
    assert_eq!(parts.len(), 1, "MBR con una partición");
    assert_eq!(parts[0].esquema, Esquema::Mbr);
    assert_eq!(parts[0].inicio, start * 512);

    let r_std = {
        let mut e = EmisorMemoria::nuevo();
        absorber(&LectorExt4::nuevo(ext.as_slice()).unwrap(), &mut e).unwrap()
    };
    let mut e = EmisorMemoria::nuevo();
    assert_eq!(absorber_particion(&datos, &parts[0], &mut e).unwrap(), r_std);

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn fs_suelto_sin_tabla() {
    if !herramientas() {
        eprintln!("SKIP: faltan herramientas");
        return;
    }
    let tmp = tmpdir("ff-bare");
    let src = tmp.join("src");
    sembrar_fat(&src);
    let fat = imagen_fat(&tmp, &src, 16);

    let parts = tabla_particiones(&fat).unwrap();
    assert_eq!(parts.len(), 1, "un FS suelto = una partición");
    assert_eq!(parts[0].esquema, Esquema::SinTabla);
    assert_eq!(parts[0].inicio, 0);
    assert_eq!(parts[0].tam, fat.len() as u64);
    assert_eq!(detectar_fs(&fat), SistemaArchivos::Fat);

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
