//! Sirve el CAS local por Akasha Over Ether (AoE). Un nodo wawa (u otro Linux)
//! puede entonces pedir cualquier blob por su hash BLAKE3 y verificarlo.
//!
//! Requiere `CAP_NET_RAW` o root (raw socket `AF_PACKET`).
//!
//! ```sh
//! sudo -E cargo run -p arje-cas-aoe --example servir_cas -- eth0 30
//! #                                                          iface  segundos (def 30)
//! ```

use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let iface = match args.next() {
        Some(i) => i,
        None => {
            eprintln!("uso: servir_cas <iface> [segundos]");
            std::process::exit(2);
        }
    };
    let segs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30);

    let n = arje_cas::list_all_shas()?.len();
    eprintln!("arje-cas-aoe :: sirviendo {n} objeto(s) del CAS por {segs}s en {iface}…");
    let stats = arje_cas_aoe::servir_cas(&iface, Duration::from_secs(segs))?;
    eprintln!(
        "arje-cas-aoe :: listo: {} servido(s), {} ignorado(s), {} fragmentado(s)",
        stats.servidos, stats.ignorados, stats.fragmentados,
    );
    Ok(())
}
