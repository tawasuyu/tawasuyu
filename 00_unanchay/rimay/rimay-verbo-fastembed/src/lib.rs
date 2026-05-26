//! `rimay-verbo-fastembed` — backend de embeddings local, sin API key,
//! sin servidor remoto.
//!
//! Usa `fastembed-rs`, que envuelve a ONNX-Runtime y descarga el modelo
//! desde Hugging Face Hub al primer arranque (cache en
//! `~/.cache/fastembed`). El default elegido es **multilingual-e5-small**:
//! 384 dimensiones, multilingüe — covera es/qu/en/otros sin tener que
//! cambiar de modelo por idioma del cuerpo en pluma-multilienzo.
//!
//! Como `Provider::embed` es async y la API de `fastembed` es sincrónica,
//! el inferer corre en `tokio::task::spawn_blocking` — el runtime async
//! no se bloquea por el CPU bound de ONNX.

#![forbid(unsafe_code)]

use std::sync::Arc;

use async_trait::async_trait;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use rimay_verbo_core::{EmbedError, EmbeddingVector, ModelId, Provider};
use tokio::sync::Mutex;

/// Provider de embeddings vía fastembed/ONNX, ejecutado en CPU.
///
/// `try_new` descarga el modelo al primer uso si no está en cache. Esa
/// descarga es bloqueante — llamala desde un contexto async sin sostener
/// locks largos, o desde el thread principal antes de levantar el
/// runtime async (el caso típico del `verbo-daemon`).
pub struct FastembedProvider {
    /// `fastembed::TextEmbedding` mantiene el modelo cargado en RAM. Un
    /// `Mutex` async serializa accesos — la API de inferencia toma
    /// `&mut self` internamente, así que dos hilos llamando `embed`
    /// concurrentemente no es seguro. Para paralelismo real, levantar
    /// dos providers o saturar `embed_batch`.
    inner: Arc<Mutex<TextEmbedding>>,
    /// `ModelId` que firma cada `EmbeddingVector` que devuelve este
    /// provider — coherente con el contrato de `rimay-verbo-core`.
    model_id: ModelId,
}

impl FastembedProvider {
    /// Construye un provider sirviendo el modelo dado. Bloquea si la
    /// descarga del modelo no estaba en cache: lo correcto es llamarlo
    /// antes de spawnear el runtime async (lo que hace el bin de
    /// `verbo-daemon`).
    pub fn try_new(modelo: EmbeddingModel) -> anyhow::Result<Self> {
        let nombre = nombre_canonico(&modelo).to_string();
        let dimension = dimension(&modelo);
        let inner = TextEmbedding::try_new(InitOptions::new(modelo))?;
        Ok(Self {
            inner: Arc::new(Mutex::new(inner)),
            model_id: ModelId::new(nombre, dimension),
        })
    }

    /// Atajo: `multilingual-e5-small`. La elección por defecto para la
    /// suite — multilingüe, 384d, descarga ligera (~120 MB ONNX).
    pub fn try_default() -> anyhow::Result<Self> {
        Self::try_new(EmbeddingModel::MultilingualE5Small)
    }
}

#[async_trait]
impl Provider for FastembedProvider {
    fn model_id(&self) -> &ModelId {
        &self.model_id
    }

    async fn embed(&self, text: &str) -> Result<EmbeddingVector, EmbedError> {
        let inner = self.inner.clone();
        let owned = text.to_string();
        let valores = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<f32>> {
            // `blocking_lock` no se permite dentro de un runtime async; usamos
            // un `try_lock` con backoff manual. Como spawn_blocking corre en
            // un thread del pool de blocking-tasks, sí es seguro bloquear ahí.
            let model = inner.blocking_lock();
            let mut salida = model.embed(vec![owned.as_str()], None)?;
            Ok(salida.pop().expect("fastembed devolvió cero vectores"))
        })
        .await
        .map_err(|e| EmbedError::Backend(format!("spawn_blocking falló: {e}")))?
        .map_err(|e| EmbedError::Backend(format!("fastembed embed: {e}")))?;
        EmbeddingVector::new(self.model_id.clone(), valores)
    }

    async fn embed_batch(
        &self,
        texts: &[String],
    ) -> Result<Vec<EmbeddingVector>, EmbedError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let inner = self.inner.clone();
        let owned: Vec<String> = texts.to_vec();
        let model_id = self.model_id.clone();
        let vectores =
            tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Vec<f32>>> {
                let model = inner.blocking_lock();
                let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
                let salida = model.embed(refs, None)?;
                Ok(salida)
            })
            .await
            .map_err(|e| EmbedError::Backend(format!("spawn_blocking falló: {e}")))?
            .map_err(|e| EmbedError::Backend(format!("fastembed embed_batch: {e}")))?;

        vectores
            .into_iter()
            .map(|v| EmbeddingVector::new(model_id.clone(), v))
            .collect::<Result<Vec<_>, EmbedError>>()
    }
}

/// Nombre canónico (string estable) de cada modelo soportado. Se anota
/// como `ModelId::name`; dos `EmbeddingVector`s del mismo modelo comparten
/// este string y son comparables vía `cosine`.
///
/// Sin pretender cubrir todos los modelos de fastembed — solo los que la
/// suite usa hoy o piensa usar pronto. Para los demás se devuelve un
/// nombre genérico que sigue funcionando, simplemente menos descriptivo.
fn nombre_canonico(modelo: &EmbeddingModel) -> &'static str {
    use EmbeddingModel::*;
    match modelo {
        MultilingualE5Small => "multilingual-e5-small",
        MultilingualE5Base => "multilingual-e5-base",
        MultilingualE5Large => "multilingual-e5-large",
        BGESmallENV15 => "bge-small-en-v1.5",
        BGEBaseENV15 => "bge-base-en-v1.5",
        BGELargeENV15 => "bge-large-en-v1.5",
        _ => "fastembed-otro",
    }
}

/// Dimensionalidad por modelo. Si fastembed añade modelos nuevos y no
/// están en este match, se devuelve 0 — `ModelId::dimension` quedaría
/// inválido y la primera llamada a `EmbeddingVector::new` lo señalaría
/// con `BadDimension`. Mejor declarar el modelo aquí cuando se incorpore.
fn dimension(modelo: &EmbeddingModel) -> usize {
    use EmbeddingModel::*;
    match modelo {
        MultilingualE5Small => 384,
        MultilingualE5Base => 768,
        MultilingualE5Large => 1024,
        BGESmallENV15 => 384,
        BGEBaseENV15 => 768,
        BGELargeENV15 => 1024,
        _ => 0,
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn nombre_canonico_es_estable() {
        assert_eq!(
            nombre_canonico(&EmbeddingModel::MultilingualE5Small),
            "multilingual-e5-small"
        );
        assert_eq!(dimension(&EmbeddingModel::MultilingualE5Small), 384);
        assert_eq!(dimension(&EmbeddingModel::MultilingualE5Base), 768);
    }

    /// Test de integración: descarga el modelo en el primer arranque
    /// (~120 MB) y verifica que vectores idénticos den coseno 1 y
    /// distintos den menos. Marcado `#[ignore]` para que `cargo test`
    /// rutinario no lo dispare — correr explícitamente:
    ///
    ///   cargo test -p rimay-verbo-fastembed -- --ignored
    #[tokio::test]
    #[ignore]
    async fn integracion_e5_small_distingue_textos() {
        let provider = FastembedProvider::try_default().expect("init e5");
        let a = provider.embed("El cóndor cruzó el cielo.").await.unwrap();
        let b = provider.embed("El cóndor cruzó el cielo.").await.unwrap();
        let c = provider
            .embed("La función de Bessel diverge en cero.")
            .await
            .unwrap();
        let ab = a.cosine(&b).unwrap();
        let ac = a.cosine(&c).unwrap();
        assert!(ab > 0.999, "idénticos: {ab}");
        assert!(ac < ab, "distintos {ac} no debería superar idénticos {ab}");
    }
}
