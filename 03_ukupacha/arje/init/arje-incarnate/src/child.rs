//! Helpers que corren EN el hijo post-clone, antes de execve.
//!
//! Reglas inviolables (la clausura de clone(2) corre en stack nuevo, COW):
//!   - sólo syscalls async-signal-safe
//!   - no `println!`/`tracing!`/cualquier I/O del runtime
//!   - no allocator (vec/box/string)
//!   - no Drop con efectos
//!   - capturar sólo Copy o datos pre-construidos

use brahman_card::ResourceLimits;

/// SAFETY: invocada en el hijo post-clone, sólo libc.
pub unsafe fn apply_rlimits(rl: &ResourceLimits) {
    if let Some(mem) = rl.mem_bytes {
        let lim = libc::rlimit {
            rlim_cur: mem,
            rlim_max: mem,
        };
        libc::setrlimit(libc::RLIMIT_AS, &lim);
    }
    if let Some(np) = rl.nproc {
        let lim = libc::rlimit {
            rlim_cur: np as u64,
            rlim_max: np as u64,
        };
        libc::setrlimit(libc::RLIMIT_NPROC, &lim);
    }
    if let Some(nf) = rl.nofile {
        let lim = libc::rlimit {
            rlim_cur: nf as u64,
            rlim_max: nf as u64,
        };
        libc::setrlimit(libc::RLIMIT_NOFILE, &lim);
    }
}

/// SAFETY: idem. `MS_PRIVATE | MS_REC` sobre `/` para que mounts del hijo
/// no se filtren al host. Trampa típica al delegar mount ns.
pub unsafe fn make_root_private() {
    libc::mount(
        std::ptr::null(),
        b"/\0".as_ptr() as *const _,
        std::ptr::null(),
        libc::MS_PRIVATE | libc::MS_REC,
        std::ptr::null(),
    );
}
