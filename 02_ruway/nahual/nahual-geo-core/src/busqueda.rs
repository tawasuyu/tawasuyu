//! Búsqueda local y soberana sobre `MapData`: ranking por nombre/propiedades,
//! tolerante a acentos y mayúsculas.

use crate::tipos::MapData;

/// Busca features cuyo nombre o propiedades casen con `query` (sin distinción
/// de mayúsculas). Ranking: igualdad > prefijo > substring; el nombre pesa
/// sobre las propiedades. Devuelve hasta `limit` índices de `data.features`.
///
/// Geocodificación local y soberana: no consulta ningún servicio externo —
/// busca dentro de lo que ya cargaste. Para buscar direcciones de medio mundo
/// alcanza con cargar un dataset (un archivo), no una API.
pub fn search(data: &MapData, query: &str, limit: usize) -> Vec<usize> {
    let q = fold(&query.trim().to_lowercase());
    if q.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(u8, usize)> = Vec::new();
    for (fi, f) in data.features.iter().enumerate() {
        // El nombre cuenta doble (peso 2×); las propiedades, simple.
        let mut best = f
            .name
            .as_deref()
            .map(|n| match_score(n, &q) * 2)
            .unwrap_or(0);
        for (_, v) in &f.props {
            best = best.max(match_score(v, &q));
        }
        if best > 0 {
            scored.push((best, fi));
        }
    }
    // Mayor puntaje primero; a igual puntaje, orden estable por índice.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().take(limit).map(|(_, fi)| fi).collect()
}

/// Puntaje de coincidencia de `q` (ya en minúsculas y sin acentos) en `s`:
/// 3 igual, 2 prefijo, 1 substring, 0 nada. Plega acentos de `s` para que
/// "peru" encuentre "Perú".
fn match_score(s: &str, q: &str) -> u8 {
    let l = fold(&s.to_lowercase());
    if l == q {
        3
    } else if l.starts_with(q) {
        2
    } else if l.contains(q) {
        1
    } else {
        0
    }
}

/// Plega acentos latinos comunes (es/pt) a su vocal base, para búsqueda
/// tolerante a tildes. No es Unicode-completo, sólo lo usual en topónimos.
fn fold(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' | 'ã' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            other => other,
        })
        .collect()
}
