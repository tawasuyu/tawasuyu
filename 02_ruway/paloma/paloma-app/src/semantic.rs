//! El motor de búsqueda por significado del binario `paloma`.
//!
//! Implementa el trait `SemanticEngine` de `paloma-llimphi` con lo que el
//! frontend no puede tener: un **runtime async** (tokio) que pega contra el
//! `rimay-verbo-daemon` por embeddings, y el `SemanticIndex` de
//! `paloma-semantic` (persistido por cuenta). El bucle de UI sigue síncrono: le
//! pasa la consulta y el corpus, y recibe los ids rankeados de vuelta por
//! `Handle::dispatch(Msg::SemanticResults)`.
//!
//! Se engancha sólo si hay un daemon corriendo (o si `PALOMA_SEMANTIC=mock`
//! para desarrollo: el mock es determinista pero **no** semántico de verdad).
//! Sin motor, el modo semántico de la UI cae a la búsqueda exacta.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use paloma_core::{Message, MessageId};
use paloma_llimphi::{Handle, Msg, SemanticEngine};
use paloma_semantic::SemanticIndex;
use rimay_verbo::Provider;
use tokio::runtime::Runtime;

/// Cuántos resultados devuelve el ranking semántico. Generoso: el panel
/// scrollea, y la cola de baja relevancia no estorba.
const TOP_K: usize = 50;

/// El motor concreto: runtime + proveedor de embeddings + índice persistido.
pub struct DaemonSemantic {
    rt: Runtime,
    provider: Arc<dyn Provider>,
    index: Arc<Mutex<SemanticIndex>>,
    index_path: PathBuf,
}

impl DaemonSemantic {
    /// Intenta construir el motor para `account_id`. Devuelve `None` si no hay
    /// daemon (y no se pidió mock) o si no se puede resolver el dir de caché —
    /// en ese caso la UI se queda en búsqueda exacta.
    pub fn try_build(account_id: &str, cache_root: Option<PathBuf>) -> Option<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .ok()?;

        let want_mock = std::env::var("PALOMA_SEMANTIC")
            .map(|v| v.eq_ignore_ascii_case("mock"))
            .unwrap_or(false);

        // Conectar al daemon; si no está, mock sólo bajo opt-in explícito.
        let provider: Arc<dyn Provider> = rt.block_on(async {
            match rimay_verbo::conectar().await {
                Ok(cliente) => Some(Arc::new(cliente) as Arc<dyn Provider>),
                Err(_) if want_mock => {
                    Some(Arc::new(rimay_verbo::MockProvider::new(384)) as Arc<dyn Provider>)
                }
                Err(_) => None,
            }
        })?;

        let model = provider.model_id().clone();
        let root = cache_root?.join("semantic");
        let index_path = root.join(format!("{}.pc", sanitize(account_id)));
        let index = SemanticIndex::load(&index_path, model).unwrap_or_else(|e| {
            eprintln!("paloma · índice semántico ilegible ({e}); arrancando vacío");
            // load ya cae a vacío salvo error de códec; ante eso, vacío del modelo.
            SemanticIndex::new(provider.model_id().clone())
        });

        Some(Self {
            rt,
            provider,
            index: Arc::new(Mutex::new(index)),
            index_path,
        })
    }
}

impl SemanticEngine for DaemonSemantic {
    fn search(&self, query: String, corpus: Vec<Message>, handle: Handle<Msg>) {
        let provider = self.provider.clone();
        let index = self.index.clone();
        let path = self.index_path.clone();

        self.rt.spawn(async move {
            // 1) Embeber lo que falte del corpus (incremental) y purgar lo que
            //    ya no está. Los locks se sueltan ANTES de cada await.
            let missing: Vec<Message> = {
                let idx = index.lock().unwrap();
                idx.missing(&corpus).into_iter().cloned().collect()
            };
            if !missing.is_empty() {
                match paloma_semantic::embed_messages(&*provider, &missing).await {
                    Ok(entries) => {
                        let keep: Vec<MessageId> = corpus.iter().map(|m| m.id.clone()).collect();
                        let mut idx = index.lock().unwrap();
                        idx.ingest(entries);
                        idx.retain(&keep);
                        if let Err(e) = idx.save(&path) {
                            eprintln!("paloma · no se pudo persistir el índice semántico: {e}");
                        }
                    }
                    Err(e) => {
                        eprintln!("paloma · falló embeber el corpus: {e}");
                        // Igual seguimos: rankeamos contra lo que haya.
                    }
                }
            }

            // 2) Embeber la consulta y rankear (sync, bajo lock breve).
            let query_vec = match paloma_semantic::embed_query(&*provider, &query).await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("paloma · falló embeber la consulta: {e}");
                    return;
                }
            };
            let ids: Vec<MessageId> = {
                let idx = index.lock().unwrap();
                match idx.search(&query_vec, TOP_K, 0.0) {
                    Ok(hits) => hits.into_iter().map(|h| h.id).collect(),
                    Err(e) => {
                        eprintln!("paloma · ranking semántico: {e}");
                        return;
                    }
                }
            };

            // 3) Devolver al bucle de UI.
            handle.dispatch(Msg::SemanticResults(ids));
        });
    }
}

/// Nombre de archivo seguro a partir del id de cuenta (un correo trae `@`/`.`).
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
