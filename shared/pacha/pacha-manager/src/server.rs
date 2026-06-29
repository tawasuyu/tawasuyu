//! El daemon: sirve el socket de control sobre un [`Manager`], persiste tras
//! cada mutación y traduce `Req` → acción → `Resp`.

use std::path::Path;
use std::sync::Arc;

use pacha_core::BringUp;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;

use crate::proto::{self, PachaInfo, Req, Resp};
use crate::{paths, Manager, Surfaces};

/// Construye la lista de contextos + su estado vivo para CLI/UI.
fn infos<S: Surfaces>(m: &Manager<S>) -> Vec<PachaInfo> {
    m.catalog
        .iter()
        .map(|p| PachaInfo {
            id: p.id.clone(),
            label: p.label.clone(),
            lifecycle: m.runtime.lifecycle(&p.id),
            active: m.runtime.active() == Some(p.id.as_str()),
        })
        .collect()
}

/// Sirve el socket hasta que el proceso termine. Reemplaza un socket viejo si
/// quedó de una corrida anterior.
pub async fn serve<S>(manager: Manager<S>, socket: &Path) -> std::io::Result<()>
where
    S: Surfaces + 'static,
{
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    let mgr = Arc::new(Mutex::new(manager));
    loop {
        let (stream, _) = listener.accept().await?;
        let mgr = mgr.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(stream, mgr).await {
                tracing::debug!(error = %e, "conexión de control terminó con error");
            }
        });
    }
}

async fn handle<S>(mut stream: UnixStream, mgr: Arc<Mutex<Manager<S>>>) -> std::io::Result<()>
where
    S: Surfaces,
{
    loop {
        let req: Req = match proto::read_frame(&mut stream).await {
            Ok(r) => r,
            // EOF/cliente cerró: salimos del loop sin ruido.
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        let resp = dispatch(&mgr, req).await;
        proto::write_frame(&mut stream, &resp).await?;
    }
}

async fn dispatch<S>(mgr: &Arc<Mutex<Manager<S>>>, req: Req) -> Resp
where
    S: Surfaces,
{
    let mut m = mgr.lock().await;
    match req {
        Req::Switch { to, fresh } => {
            let bring = if fresh { BringUp::Fresh } else { BringUp::Restore };
            match m.switch(&to, bring).await {
                Ok(warnings) => {
                    if let Err(e) = paths::save_runtime(&m.runtime) {
                        tracing::warn!(error = %e, "no pude persistir state.ron");
                    }
                    Resp::Switched { active: m.runtime.active().map(str::to_string), warnings }
                }
                Err(e) => Resp::Err(e.to_string()),
            }
        }
        Req::Close { id } => match m.close(&id).await {
            Ok(warnings) => {
                let _ = paths::save_runtime(&m.runtime);
                Resp::Switched { active: m.runtime.active().map(str::to_string), warnings }
            }
            Err(e) => Resp::Err(e.to_string()),
        },
        Req::List => Resp::List(infos(&m)),
        Req::Status => Resp::List(infos(&m)),
        Req::Define(p) => {
            m.catalog.upsert(*p);
            match paths::save_catalog(&m.catalog) {
                Ok(()) => Resp::Ok,
                Err(e) => Resp::Err(e.to_string()),
            }
        }
        Req::Remove { id } => {
            m.catalog.remove(&id);
            match paths::save_catalog(&m.catalog) {
                Ok(()) => Resp::Ok,
                Err(e) => Resp::Err(e.to_string()),
            }
        }
    }
}
