//! Binario del daemon willay: abre el índice, bindea el socket Unix y sirve.

use std::fs;
use std::os::unix::net::UnixListener;

use willay_emit::socket_path;
use willay_store::Indice;

fn main() -> anyhow::Result<()> {
    // Índice persistente; si el disco falla, cae a uno efímero en memoria (el
    // daemon sigue sirviendo aunque no persista, igual que pata-notify).
    let indice = Indice::open().or_else(|_| {
        eprintln!("willay-daemon · índice en disco no disponible; usando memoria");
        Indice::temporary()
    })?;

    let path = socket_path();
    // Sacamos un socket viejo (de una instancia muerta) antes de bindear.
    let _ = fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    eprintln!("willay-daemon · escuchando en {} ({} eventos)", path.display(), indice.len());

    willay_daemon::servir(listener, indice);
    Ok(())
}
