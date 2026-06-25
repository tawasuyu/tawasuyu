//! Integration test del modo `to-partition` contra un tmpdir que pretende
//! ser una ESP. No registra NVRAM — eso requeriría privilegios y un sistema
//! UEFI real. Validamos el lado *file system* del installer:
//!
//! - layout: kernel + initramfs + seed + cmdline.txt en `<esp>/EFI/arje/`
//! - loader entries: `loader/entries/arje.conf` con title/linux/initrd/options
//! - `loader/loader.conf` con default + timeout
//! - el initramfs gzipeado es bien-formado (header gzip + cpio newc adentro)

use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root no resoluble")
}

fn installer_bin() -> PathBuf {
    // El test corre con `cargo test`, así que CARGO_BIN_EXE_<name> apunta
    // al binario recién buildeado.
    PathBuf::from(env!("CARGO_BIN_EXE_arje-installer"))
}

#[test]
fn to_partition_arma_layout_completo() {
    let esp = tempfile::tempdir().expect("tempdir ESP");
    let bins_dir = tempfile::tempdir().expect("tempdir bins");

    // Binarios stub — para el smoke test no necesitamos que sean ejecutables
    // de verdad, sólo bytes que el packager copie al initramfs.
    let arje_zero = bins_dir.path().join("arje-zero");
    let agetty = bins_dir.path().join("agetty");
    // El seed qemu declara arje-splash como genesis Native (desde 2026-06-23):
    // el packager exige un binario por cada Ente Native, así que lo stubeamos.
    let splash = bins_dir.path().join("arje-splash");
    std::fs::write(&arje_zero, b"#!/bin/sh\necho arje-zero stub\n").unwrap();
    std::fs::write(&agetty, b"#!/bin/sh\necho agetty stub\n").unwrap();
    std::fs::write(&splash, b"#!/bin/sh\necho arje-splash stub\n").unwrap();

    // "Kernel" stub — bytes arbitrarios que el installer copia tal cual a
    // <esp>/EFI/arje/vmlinuz.
    let kernel_path = bins_dir.path().join("vmlinuz-test");
    std::fs::write(&kernel_path, b"FAKE_KERNEL_BYTES_FOR_TEST\n").unwrap();

    let seed_path = workspace_root()
        .join("03_ukupacha/arje/seeds/arje-qemu.card.json");
    assert!(seed_path.is_file(), "seed canónica no existe en {}", seed_path.display());

    let st = Command::new(installer_bin())
        .arg("to-partition")
        .args([
            "--esp",
            esp.path().to_str().unwrap(),
            "--kernel",
            kernel_path.to_str().unwrap(),
            "--seed",
            seed_path.to_str().unwrap(),
            "--bin",
            &format!("arje-zero={}", arje_zero.to_str().unwrap()),
            "--bin",
            &format!("agetty-ttyS0={}", agetty.to_str().unwrap()),
            "--bin",
            &format!("arje-splash={}", splash.to_str().unwrap()),
            "--cmdline",
            "console=ttyS0 panic=10",
            "--label",
            "arje-test",
        ])
        .status()
        .expect("spawn arje-installer");
    assert!(st.success(), "installer falló con {st}");

    // Layout esperado.
    let arje_dir = esp.path().join("EFI/arje");
    assert!(arje_dir.is_dir(), "no se creó {}", arje_dir.display());
    assert!(arje_dir.join("vmlinuz").is_file(), "falta vmlinuz");
    assert!(arje_dir.join("initramfs.cpio.gz").is_file(), "falta initramfs");
    assert!(arje_dir.join("seed.card.json").is_file(), "falta seed");
    assert!(arje_dir.join("cmdline.txt").is_file(), "falta cmdline.txt");

    // Contenido del kernel: copiado tal cual.
    assert_eq!(
        std::fs::read(arje_dir.join("vmlinuz")).unwrap(),
        b"FAKE_KERNEL_BYTES_FOR_TEST\n"
    );

    // cmdline canónico.
    let cmdline = std::fs::read_to_string(arje_dir.join("cmdline.txt")).unwrap();
    assert!(cmdline.contains(r"initrd=\EFI\arje\initramfs.cpio.gz"));
    assert!(cmdline.contains("console=ttyS0"));
    assert!(cmdline.contains("panic=10"));

    // Initramfs: header gzip + identificable como cpio newc adentro.
    let gz = std::fs::read(arje_dir.join("initramfs.cpio.gz")).unwrap();
    assert_eq!(&gz[..2], &[0x1f, 0x8b], "no es gzip");
    use flate2::read::GzDecoder;
    use std::io::Read;
    let mut dec = GzDecoder::new(&gz[..]);
    let mut cpio = Vec::new();
    dec.read_to_end(&mut cpio).unwrap();
    assert_eq!(&cpio[..6], b"070701", "no es cpio newc");
    // El seed embebido en /ente/seed.card.json debe traer el label correcto.
    assert!(
        cpio.windows(b"ente/seed.card.json".len())
            .any(|w| w == b"ente/seed.card.json"),
        "no se encontró ente/seed.card.json en el initramfs"
    );

    // Loader entry.
    let entry = std::fs::read_to_string(esp.path().join("loader/entries/arje.conf"))
        .expect("falta arje.conf");
    assert!(entry.contains("title    arje-test"), "{entry}");
    assert!(entry.contains("linux    /EFI/arje/vmlinuz"), "{entry}");
    assert!(entry.contains("initrd   /EFI/arje/initramfs.cpio.gz"), "{entry}");
    let opts = entry.lines().find(|l| l.starts_with("options")).unwrap();
    assert!(opts.contains("console=ttyS0"));
    assert!(opts.contains("panic=10"));
    assert!(!opts.contains("initrd="));

    // Loader global.
    let loader = std::fs::read_to_string(esp.path().join("loader/loader.conf")).unwrap();
    assert!(loader.contains("default arje"));
    assert!(loader.contains("timeout 3"));
}

#[test]
fn to_partition_firma_attest_con_rootkey() {
    let esp = tempfile::tempdir().expect("tempdir ESP");
    let bins_dir = tempfile::tempdir().expect("tempdir bins");

    let arje_zero = bins_dir.path().join("arje-zero");
    let agetty = bins_dir.path().join("agetty");
    let splash = bins_dir.path().join("arje-splash");
    let kernel_path = bins_dir.path().join("vmlinuz-test");
    std::fs::write(&arje_zero, b"arje-zero stub bytes").unwrap();
    std::fs::write(&agetty, b"agetty stub bytes").unwrap();
    std::fs::write(&splash, b"arje-splash stub bytes").unwrap();
    std::fs::write(&kernel_path, b"KERNEL\n").unwrap();

    let seed_path = workspace_root().join("03_ukupacha/arje/seeds/arje-qemu.card.json");
    let rootkey = bins_dir.path().join("rootkey");

    let st = Command::new(installer_bin())
        .arg("to-partition")
        .args([
            "--esp", esp.path().to_str().unwrap(),
            "--kernel", kernel_path.to_str().unwrap(),
            "--seed", seed_path.to_str().unwrap(),
            "--bin", &format!("arje-zero={}", arje_zero.to_str().unwrap()),
            "--bin", &format!("agetty-ttyS0={}", agetty.to_str().unwrap()),
            "--bin", &format!("arje-splash={}", splash.to_str().unwrap()),
            "--rootkey", rootkey.to_str().unwrap(),
            "--gen-rootkey",
        ])
        .status()
        .expect("spawn arje-installer");
    assert!(st.success(), "installer con --rootkey falló con {st}");

    // La rootkey se generó (32 bytes raw).
    let key = std::fs::read(&rootkey).expect("rootkey no generada");
    assert_eq!(key.len(), 32, "la rootkey debe ser 32 bytes");

    // El seed instalado en la ESP trae el manifiesto firmado: concesiones
    // (campo `bytecode`) + `attest_rootkey` con valor (no null).
    let seed_str = std::fs::read_to_string(esp.path().join("EFI/arje/seed.card.json"))
        .expect("falta seed en la ESP");
    assert!(seed_str.contains("\"bytecode\""), "el seed instalado no tiene concesiones firmadas");
    assert!(
        seed_str.contains("\"attest_rootkey\""),
        "el seed instalado no ancla la rootkey",
    );
    assert!(
        !seed_str.contains("\"attest_rootkey\": null"),
        "attest_rootkey quedó null pese a --rootkey",
    );
}

#[test]
fn to_partition_cosecha_binarios_al_cas() {
    let esp = tempfile::tempdir().expect("tempdir ESP");
    let bins_dir = tempfile::tempdir().expect("tempdir bins");
    let cas_dir = tempfile::tempdir().expect("tempdir CAS");

    let arje_zero = bins_dir.path().join("arje-zero");
    let agetty = bins_dir.path().join("agetty");
    let splash = bins_dir.path().join("arje-splash");
    let kernel_path = bins_dir.path().join("vmlinuz-test");
    // Contenidos distintos → 3 blobs distintos en el CAS (sin dedup).
    std::fs::write(&arje_zero, b"arje-zero harvest bytes").unwrap();
    std::fs::write(&agetty, b"agetty harvest bytes").unwrap();
    std::fs::write(&splash, b"arje-splash harvest bytes").unwrap();
    std::fs::write(&kernel_path, b"KERNEL\n").unwrap();

    let seed_path = workspace_root().join("03_ukupacha/arje/seeds/arje-qemu.card.json");

    let st = Command::new(installer_bin())
        // CAS aislado al subproceso vía env — sin tocar el real ni otros tests.
        .env("ENTE_CAS_ROOT", cas_dir.path())
        .arg("to-partition")
        .args([
            "--esp", esp.path().to_str().unwrap(),
            "--kernel", kernel_path.to_str().unwrap(),
            "--seed", seed_path.to_str().unwrap(),
            "--bin", &format!("arje-zero={}", arje_zero.to_str().unwrap()),
            "--bin", &format!("agetty-ttyS0={}", agetty.to_str().unwrap()),
            "--bin", &format!("arje-splash={}", splash.to_str().unwrap()),
            "--harvest-cas",
        ])
        .status()
        .expect("spawn arje-installer");
    assert!(st.success(), "installer con --harvest-cas falló con {st}");

    // El CAS contiene los 3 binarios cosechados, cada uno con nombre = 64 hex.
    let blobs: Vec<_> = std::fs::read_dir(cas_dir.path())
        .unwrap()
        .flatten()
        .filter(|e| {
            let n = e.file_name();
            n.to_str().map(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(blobs.len(), 3, "deberían cosecharse 3 binarios distintos al CAS");
}

#[test]
fn to_partition_falla_grácil_si_falta_bin() {
    let esp = tempfile::tempdir().unwrap();
    let bins_dir = tempfile::tempdir().unwrap();
    let arje_zero = bins_dir.path().join("arje-zero");
    std::fs::write(&arje_zero, b"stub").unwrap();
    let kernel_path = bins_dir.path().join("vmlinuz-test");
    std::fs::write(&kernel_path, b"kfake").unwrap();
    let seed_path = workspace_root().join("03_ukupacha/arje/seeds/arje-qemu.card.json");

    // Falta --bin agetty-ttyS0 — el genesis del seed lo exige.
    let out = Command::new(installer_bin())
        .arg("to-partition")
        .args([
            "--esp",
            esp.path().to_str().unwrap(),
            "--kernel",
            kernel_path.to_str().unwrap(),
            "--seed",
            seed_path.to_str().unwrap(),
            "--bin",
            &format!("arje-zero={}", arje_zero.to_str().unwrap()),
        ])
        .output()
        .unwrap();
    assert!(!out.status.success(), "el installer no debería haber pasado");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("agetty-ttyS0"),
        "el error no menciona el bin faltante: {stderr}"
    );
}

#[test]
fn to_usb_sin_yes_destroy_aborta_sin_tocar_disco() {
    let bins_dir = tempfile::tempdir().unwrap();
    let arje_zero = bins_dir.path().join("arje-zero");
    std::fs::write(&arje_zero, b"stub").unwrap();
    let agetty = bins_dir.path().join("agetty");
    std::fs::write(&agetty, b"stub").unwrap();
    let kernel_path = bins_dir.path().join("vmlinuz-test");
    std::fs::write(&kernel_path, b"k").unwrap();
    let seed_path = workspace_root().join("03_ukupacha/arje/seeds/arje-qemu.card.json");

    // /dev/null es un path real, pero el installer tiene que ABORT antes
    // de tocarlo porque no pasamos --yes-destroy.
    let out = Command::new(installer_bin())
        .arg("to-usb")
        .args([
            "--device",
            "/dev/null",
            "--kernel",
            kernel_path.to_str().unwrap(),
            "--seed",
            seed_path.to_str().unwrap(),
            "--bin",
            &format!("arje-zero={}", arje_zero.to_str().unwrap()),
            "--bin",
            &format!("agetty-ttyS0={}", agetty.to_str().unwrap()),
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--yes-destroy"), "{stderr}");
}
