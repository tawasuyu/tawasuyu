//! `arje-getty-stub` — agetty mínimo para validar el ciclo de vida del
//! fractal. NO es una getty real (no negocia baudrate, no parsea /etc/issue,
//! no hace login). Sólo:
//!
//! 1. Abre la TTY pasada como primer arg (p. ej. `ttyS0` o `tty1`) bajo
//!    `/dev/`, en modo lectura+escritura.
//! 2. Re-duplica el fd a stdin/stdout/stderr — sin esto los `println!` van
//!    al stderr heredado de arje-zero y no aparecen en la consola del usuario.
//! 3. Imprime un banner que demuestra que arje-zero forkeó+execeó al hijo.
//! 4. Lee del TTY hasta EOF — bloqueando indefinidamente en QEMU sin
//!    teclado, pero respondiendo si el usuario tipea algo.
//!
//! Compila estático sin dependencias C (sólo std + libc del rustc) — listo
//! para meter en un initramfs.
//!
//! ## Por qué este crate existe
//!
//! El `agetty` real de util-linux está linkeado a glibc dinámica. Meterlo
//! en un initramfs requiere bundle del dynamic loader + libc.so + un montón
//! de objetos más, o disponer de un agetty estático (no es fácil de
//! conseguir en distros modernas). Para el smoke test del boot chain nos
//! basta un proceso que abra la TTY y prove vida.

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    // El seed canónico pasa `["--noclear", "ttyS0", "linux"]`. Encontramos
    // el primer arg que parezca un nombre de TTY — empieza con "tty".
    let tty_name = argv
        .iter()
        .skip(1)
        .find(|a| a.starts_with("tty"))
        .cloned()
        .unwrap_or_else(|| "console".to_string());

    match run(&tty_name) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // Sin TTY útil — al menos volcamos al stderr heredado para que
            // el log del padre lo capture.
            eprintln!("arje-getty-stub :: ERROR {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(tty_name: &str) -> std::io::Result<()> {
    let path = format!("/dev/{tty_name}");
    let tty = OpenOptions::new().read(true).write(true).open(&path)?;
    // Dup el fd a 0/1/2 — la convención unix para que cualquier print
    // posterior vaya a la TTY abierta.
    let fd = tty.as_raw_fd();
    unsafe {
        if libc_dup2(fd, 0) < 0 || libc_dup2(fd, 1) < 0 || libc_dup2(fd, 2) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }

    let mut stdout = std::io::stdout();
    writeln!(stdout, "\n\n=================================================")?;
    writeln!(stdout, "  arje-getty-stub :: vivo en {tty_name}")?;
    writeln!(stdout, "  fractal arje arrancó hasta este hijo")?;
    writeln!(stdout, "  pid {} ppid {}", std::process::id(), getppid_raw())?;
    writeln!(stdout, "=================================================\n")?;
    stdout.flush()?;

    // Bloquea leyendo de la TTY. En QEMU sin teclado lee 0 bytes / espera.
    let mut buf = [0u8; 1];
    loop {
        match std::io::stdin().read(&mut buf) {
            Ok(0) => return Ok(()), // EOF — la consola se cerró
            Ok(_) => {
                // Eco para confirmar interacción.
                let _ = stdout.write_all(&buf);
                let _ = stdout.flush();
            }
            Err(e) => return Err(e),
        }
    }
}

/// Mini wrapper sobre dup2(2). Sin crate `libc` para mantener el crate sin
/// dependencias externas — sólo un extern "C" inline.
unsafe fn libc_dup2(old: i32, new: i32) -> i32 {
    extern "C" {
        fn dup2(oldfd: i32, newfd: i32) -> i32;
    }
    dup2(old, new)
}

fn getppid_raw() -> i32 {
    extern "C" {
        fn getppid() -> i32;
    }
    unsafe { getppid() }
}
