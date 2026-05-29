// =============================================================================
//  ayni :: ayni-index — búsqueda semántica local del historial
// -----------------------------------------------------------------------------
//  Un índice en memoria: id de nodo (BLAKE3) → vector de embedding. Indexar un
//  mensaje es embeberlo; buscar es embeber la consulta y rankear por coseno.
//  Todo local —dueño de tus bytes—: el `Provider` es de rimay (Mock offline o
//  el daemon en producción), y nada del historial sale de la máquina.
//
//  Sólo indexa cargas de TEXTO PLANO: un nodo `Carga::Cifrado` no se puede
//  embeber sin descifrar, y descifrar es decisión de la app (que tiene el
//  canal), no del índice. Quien quiera buscar en mensajes cifrados los descifra
//  primero y pasa el claro a [`Indice::indexar`].
// =============================================================================

use std::collections::HashMap;

use ayni_core::{Conversacion, Hash};
use rimay_verbo_core::{EmbedError, EmbeddingVector, Provider};

/// Índice semántico: vector por id de nodo. En memoria; el `EmbeddingVector` es
/// `Serialize`, así que persistirlo (junto al store) es trivial cuando haga falta.
#[derive(Default)]
pub struct Indice {
    vectores: HashMap<Hash, EmbeddingVector>,
}

impl Indice {
    /// Un índice vacío.
    pub fn nuevo() -> Self {
        Indice::default()
    }

    /// Cuántos nodos hay indexados.
    pub fn len(&self) -> usize {
        self.vectores.len()
    }

    /// ¿Índice vacío?
    pub fn esta_vacio(&self) -> bool {
        self.vectores.is_empty()
    }

    /// ¿Está este nodo indexado?
    pub fn contiene(&self, id: &Hash) -> bool {
        self.vectores.contains_key(id)
    }

    /// Indexa un mensaje: embebe su texto y guarda el vector bajo el id del nodo.
    /// Reindexar el mismo id reemplaza su vector.
    pub async fn indexar(
        &mut self,
        id: Hash,
        texto: &str,
        provider: &dyn Provider,
    ) -> Result<(), EmbedError> {
        let vector = provider.embed(texto).await?;
        self.vectores.insert(id, vector);
        Ok(())
    }

    /// Indexa todos los nodos de TEXTO PLANO de una conversación que aún no
    /// estén en el índice. Los nodos cifrados se omiten (no se pueden embeber
    /// sin descifrar). Devuelve cuántos se indexaron en esta pasada.
    pub async fn indexar_conversacion(
        &mut self,
        conv: &Conversacion,
        provider: &dyn Provider,
    ) -> Result<usize, EmbedError> {
        let mut n = 0;
        for (id, nodo) in conv.nodos() {
            if self.vectores.contains_key(id) {
                continue;
            }
            if let Some(texto) = nodo.contenido.carga.texto() {
                self.indexar(*id, texto, provider).await?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// Olvida un nodo del índice.
    pub fn olvidar(&mut self, id: &Hash) {
        self.vectores.remove(id);
    }

    /// Busca los `k` nodos más parecidos a la consulta, por similitud coseno,
    /// de mayor a menor. Embebe la consulta con el mismo `provider` (mismo
    /// espacio vectorial) y compara contra todo lo indexado.
    pub async fn buscar(
        &self,
        consulta: &str,
        provider: &dyn Provider,
        k: usize,
    ) -> Result<Vec<(Hash, f32)>, EmbedError> {
        let q = provider.embed(consulta).await?;
        let mut puntuados: Vec<(Hash, f32)> = Vec::with_capacity(self.vectores.len());
        for (id, vector) in &self.vectores {
            // coseno falla sólo si los modelos difieren; con un provider único
            // eso no pasa, pero si pasara, ese nodo simplemente no se rankea.
            if let Ok(sim) = q.cosine(vector) {
                puntuados.push((*id, sim));
            }
        }
        puntuados.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        puntuados.truncate(k);
        Ok(puntuados)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rimay_verbo_mock::MockProvider;

    #[tokio::test]
    async fn la_consulta_exacta_rankea_primero() {
        let p = MockProvider::default(); // 384d, determinista
        let mut idx = Indice::nuevo();

        // ids arbitrarios distintos para tres mensajes.
        let id_a = [1u8; 32];
        let id_b = [2u8; 32];
        let id_c = [3u8; 32];
        idx.indexar(id_a, "vamos por un café mañana", &p).await.unwrap();
        idx.indexar(id_b, "el informe trimestral está listo", &p).await.unwrap();
        idx.indexar(id_c, "nos vemos en la estación", &p).await.unwrap();
        assert_eq!(idx.len(), 3);

        // buscar el texto EXACTO de B: su vector coincide consigo mismo (coseno
        // ≈ 1.0), así que debe encabezar el ranking.
        let resultados = idx.buscar("el informe trimestral está listo", &p, 3).await.unwrap();
        assert_eq!(resultados[0].0, id_b, "la coincidencia exacta encabeza");
        assert!(resultados[0].1 > 0.999, "coseno ≈ 1.0 consigo mismo");
        assert_eq!(resultados.len(), 3);
    }

    #[tokio::test]
    async fn indexa_conversacion_y_omite_cifrados() {
        use ayni_core::{Carga, Conversacion};
        // identidad mínima sin ayni-crypto: firmamos con un closure trivial
        // (el índice no verifica firmas; sólo lee la carga de texto).
        let p = MockProvider::default();
        let autor = [7u8; 32];
        let firma_falsa = |_: &Hash| [0u8; 64];

        let mut conv = Conversacion::nueva();
        let n1 = conv.redactar(autor, Carga::Texto("hola".into()), 1, firma_falsa);
        conv.agregar(n1).unwrap();
        let n2 = conv.redactar(autor, Carga::Cifrado(vec![1, 2, 3]), 2, firma_falsa);
        conv.agregar(n2).unwrap();

        let mut idx = Indice::nuevo();
        let indexados = idx.indexar_conversacion(&conv, &p).await.unwrap();
        assert_eq!(indexados, 1, "sólo el de texto plano; el cifrado se omite");
        assert_eq!(idx.len(), 1);

        // reindexar no duplica.
        let otra_vez = idx.indexar_conversacion(&conv, &p).await.unwrap();
        assert_eq!(otra_vez, 0);
    }
}
