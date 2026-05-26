//! Roundtrip de un cpio.gz armado por la lib contra el `cpio` y `gunzip`
//! del host. Si pasa, el archive es interoperable con cualquier kernel
//! Linux que monta initramfs.
//!
//! Por qué un test de integración: validar el binario de salida contra
//! una implementación independiente es la única forma de cazar bugs sutiles
//! del formato newc (alineación, mtime, ino) que `cargo test` unit no detecta.
//!
//! Si `cpio` o `gunzip` no están en el host, el test se salta — no
//! queremos bloquear CI en máquinas sin esas tools. En el host de desarrollo
//! del fractal arje ambos están instalados por defecto.

use std::io::Write;
use std::process::{Command, Stdio};

use arje_packager::{gzip, CpioWriter, EntryKind};

fn have_tool(name: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {name}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn cpio_tv_lee_el_archive_de_la_lib() {
    if !have_tool("cpio") {
        eprintln!("skip: cpio no instalado en el host");
        return;
    }

    let mut w = CpioWriter::new(Vec::new());
    w.append("sbin", EntryKind::Directory).unwrap();
    w.append(
        "sbin/saludo",
        EntryKind::Regular { data: b"hola fractal", perm: 0o755 },
    )
    .unwrap();
    w.append("init", EntryKind::Symlink { target: "sbin/saludo" }).unwrap();
    let buf = w.finish().unwrap();

    // `cpio -tv --format=newc` rechaza el archive si el formato no cierra.
    let mut child = Command::new("cpio")
        .arg("-t")
        .arg("--quiet")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cpio");
    child.stdin.as_mut().unwrap().write_all(&buf).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "cpio -t falló: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let listing = String::from_utf8_lossy(&out.stdout);
    assert!(listing.contains("sbin/saludo"), "no apareció saludo: {listing}");
    assert!(listing.contains("init"), "no apareció init: {listing}");
    assert!(listing.contains("sbin\n") || listing.starts_with("sbin"), "no apareció dir sbin: {listing}");
}

#[test]
fn gunzip_recupera_el_cpio_intacto() {
    if !have_tool("gunzip") {
        eprintln!("skip: gunzip no instalado en el host");
        return;
    }

    let mut w = CpioWriter::new(Vec::new());
    w.append(
        "ente/seed.card.json",
        EntryKind::Regular { data: br#"{"label":"test"}"#, perm: 0o644 },
    )
    .unwrap();
    let cpio_bytes = w.finish().unwrap();
    let gz = gzip(&cpio_bytes).unwrap();

    let mut child = Command::new("gunzip")
        .arg("-c")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn gunzip");
    child.stdin.as_mut().unwrap().write_all(&gz).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "gunzip falló");
    assert_eq!(
        out.stdout, cpio_bytes,
        "gunzip(gzip(x)) != x — corrupción en el pipeline de compresión"
    );
}

#[test]
fn extracted_files_tienen_el_contenido_esperado() {
    if !have_tool("cpio") {
        eprintln!("skip: cpio no instalado en el host");
        return;
    }

    let payload = b"contenido idempotente del Ente saludador";

    let mut w = CpioWriter::new(Vec::new());
    w.append("usr", EntryKind::Directory).unwrap();
    w.append("usr/bin", EntryKind::Directory).unwrap();
    w.append(
        "usr/bin/saludo",
        EntryKind::Regular { data: payload, perm: 0o755 },
    )
    .unwrap();
    let buf = w.finish().unwrap();

    let tmp = tempfile::tempdir().unwrap();
    // `cpio -i` extrae en CWD, así que entramos al tmp y se lo damos por stdin.
    let mut child = Command::new("cpio")
        .current_dir(tmp.path())
        .arg("-id")
        .arg("--quiet")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cpio -id");
    child.stdin.as_mut().unwrap().write_all(&buf).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "cpio -id falló: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let extracted = std::fs::read(tmp.path().join("usr/bin/saludo")).unwrap();
    assert_eq!(extracted, payload);
}
