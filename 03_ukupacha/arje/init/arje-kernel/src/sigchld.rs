//! SIGCHLD vía signalfd, no signal handler.
//!
//! Los handlers async-signal sólo pueden invocar funciones async-signal-safe
//! — no allocator, no `mpsc::send`. Con signalfd la señal entra al runtime de
//! Tokio como un `fd` legible y la cosechamos en el bucle como cualquier otro
//! evento. Esto es lo que hace que un init en Rust moderno sea sano.

use anyhow::Context;
use nix::sys::signal::Signal;
use nix::sys::signalfd::{SfdFlags, SigSet, SignalFd};
use std::os::fd::AsRawFd;
use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc;
use tracing::{error, trace};

/// Bloquea SIGCHLD para entrega asíncrona, abre signalfd, y emite un `()`
/// en el canal cada vez que llega al menos una señal.
pub fn spawn_sigchld_stream() -> anyhow::Result<mpsc::Receiver<()>> {
    let mut mask = SigSet::empty();
    mask.add(Signal::SIGCHLD);
    mask.thread_block().context("SIGCHLD thread_block")?;

    let sfd = SignalFd::with_flags(&mask, SfdFlags::SFD_NONBLOCK | SfdFlags::SFD_CLOEXEC)
        .context("signalfd creation")?;

    let async_fd = AsyncFd::new(SignalFdHandle(sfd)).context("AsyncFd::new")?;

    let (tx, rx) = mpsc::channel(8);
    tokio::spawn(async move {
        loop {
            let mut guard = match async_fd.readable().await {
                Ok(g) => g,
                Err(e) => { error!(?e, "signalfd readable failed"); return; }
            };
            // Drenamos todas las siginfos pendientes; signalfd las coalesce
            // pero no las cuenta — un read por evento legible es suficiente.
            drain(guard.get_inner());
            guard.clear_ready();
            if tx.send(()).await.is_err() { return; }
            trace!("SIGCHLD batch coalesced");
        }
    });

    Ok(rx)
}

struct SignalFdHandle(SignalFd);

impl AsRawFd for SignalFdHandle {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.0.as_raw_fd()
    }
}

fn drain(handle: &SignalFdHandle) {
    let fd = handle.as_raw_fd();
    // Tamaño exacto de signalfd_siginfo. Leemos en bucle hasta EAGAIN.
    let mut buf = [0u8; std::mem::size_of::<libc::signalfd_siginfo>()];
    loop {
        let n = unsafe {
            libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len())
        };
        if n < 0 { return; }
        if n == 0 { return; }
    }
}
