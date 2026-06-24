//! Atlas offline de ciudades para el picker de lugar de nacimiento.
//!
//! Fuente: **GeoNames `cities15000`** (todas las ciudades con población
//! ≥ 15.000 — ~34k en todo el mundo), reducido a un TSV compacto
//! `name\tCC\tlat\tlon\tpop\ttz` ordenado por población descendente y
//! embebido con `include_str!`. Se parsea una vez (`LazyLock`) a un
//! `Vec<City>` con campos `&'static str` que tajan el string embebido —
//! sin asignaciones por ciudad.
//!
//! Por qué offline: la suite es local-first y soberana (wawa ni siquiera
//! tiene TCP/IP). Geocodificar contra una API pública filtraría las
//! consultas de **lugar de nacimiento** a un tercero y ataría cosmos a la
//! red. Acá todo resuelve en proceso.
//!
//! La **zona horaria** se guarda como nombre IANA (`America/Lima`,
//! `Europe/Madrid`, …); el offset correcto —con el DST que regía en la
//! época— se computa con [`offset_minutes_at`] al instante de nacimiento,
//! no se hardcodea. Eso corrige cartas de fechas con reglas de horario de
//! verano hoy extintas.

#![forbid(unsafe_code)]

use std::sync::LazyLock;

use chrono::{NaiveDateTime, Offset, TimeZone};
use chrono_tz::Tz;

/// Una ciudad del atlas. Los `&'static str` tajan el TSV embebido.
#[derive(Debug, Clone, Copy)]
pub struct City {
    /// Nombre (en su grafía local, p.ej. `Córdoba`, `München`).
    pub name: &'static str,
    /// Código de país ISO-3166-1 alfa-2 (`AR`, `ES`, `US`).
    pub country: &'static str,
    pub lat: f64,
    pub lon: f64,
    pub population: u32,
    /// Nombre de zona IANA (`America/Argentina/Cordoba`).
    pub tz: &'static str,
}

impl City {
    /// Etiqueta amigable para mostrar/guardar: `«Nombre, CC»`.
    pub fn label(&self) -> String {
        format!("{}, {}", self.name, self.country)
    }

    /// Offset respecto de UTC en minutos en `local` (fecha+hora locales),
    /// con el DST que regía en esa fecha. `None` si la zona no parsea o la
    /// hora local es ambigua/inexistente por un salto de DST.
    pub fn offset_minutes_at(&self, local: NaiveDateTime) -> Option<i32> {
        offset_minutes_at(self.tz, local)
    }
}

static DATA: &str = include_str!("../data/cities.tsv");

static CITIES: LazyLock<Vec<City>> = LazyLock::new(|| DATA.lines().filter_map(parse_line).collect());

fn parse_line(line: &'static str) -> Option<City> {
    let mut it = line.split('\t');
    let name = it.next()?;
    let country = it.next()?;
    let lat = it.next()?.parse().ok()?;
    let lon = it.next()?.parse().ok()?;
    let population = it.next()?.parse().unwrap_or(0);
    let tz = it.next()?;
    if name.is_empty() || tz.is_empty() {
        return None;
    }
    Some(City { name, country, lat, lon, population, tz })
}

/// Todas las ciudades, ordenadas por población descendente.
pub fn all() -> &'static [City] {
    &CITIES
}

/// Normaliza para comparar: minúsculas + pliega los diacríticos latinos
/// comunes a ASCII (`Córdoba` → `cordoba`, `São Paulo` → `sao paulo`),
/// así «cordoba» matchea «Córdoba» sin tildes.
fn normalize(s: &str) -> String {
    s.chars()
        .flat_map(|c| fold_char(c.to_ascii_lowercase().to_string().chars().next().unwrap_or(c)))
        .collect()
}

fn fold_char(c: char) -> std::vec::IntoIter<char> {
    let folded: &[char] = match c {
        'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => &['a'],
        'é' | 'è' | 'ë' | 'ê' => &['e'],
        'í' | 'ì' | 'ï' | 'î' => &['i'],
        'ó' | 'ò' | 'ö' | 'ô' | 'õ' => &['o'],
        'ú' | 'ù' | 'ü' | 'û' => &['u'],
        'ñ' => &['n'],
        'ç' => &['c'],
        'ß' => &['s', 's'],
        other => return vec![other].into_iter(),
    };
    folded.to_vec().into_iter()
}

/// Búsqueda fuzzy por nombre (case- y diacrítico-insensible), rankeada por
/// calidad de match (exacto > prefijo > substring) y, dentro de cada
/// rango, por población descendente (la data ya viene ordenada así, y el
/// sort es estable). Query vacía ⇒ las más pobladas del mundo. Hasta
/// `limit` resultados.
pub fn search(query: &str, limit: usize) -> Vec<&'static City> {
    let q = normalize(query);
    if q.is_empty() {
        return CITIES.iter().take(limit).collect();
    }
    let mut hits: Vec<(u8, &'static City)> = CITIES
        .iter()
        .filter_map(|c| {
            let n = normalize(c.name);
            let rank = if n == q {
                0
            } else if n.starts_with(&q) {
                1
            } else if n.contains(&q) {
                2
            } else {
                return None;
            };
            Some((rank, c))
        })
        .collect();
    hits.sort_by_key(|(rank, _)| *rank); // estable: preserva el orden por población
    hits.into_iter().take(limit).map(|(_, c)| c).collect()
}

/// Offset respecto de UTC en minutos para una zona IANA en una fecha/hora
/// locales dadas, con el DST que regía en esa fecha. `None` si la zona no
/// parsea o la hora local cae en un salto de DST.
pub fn offset_minutes_at(tz_iana: &str, local: NaiveDateTime) -> Option<i32> {
    let tz: Tz = tz_iana.parse().ok()?;
    let dt = tz.from_local_datetime(&local).earliest()?;
    Some(dt.offset().fix().local_minus_utc() / 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn dataset_no_vacio_y_ordenado_por_poblacion() {
        let all = all();
        assert!(all.len() > 20_000, "esperaba ~34k ciudades, hay {}", all.len());
        // Ordenado por población descendente.
        assert!(all[0].population >= all[100].population);
    }

    #[test]
    fn busca_sin_tildes() {
        let r = search("cordoba", 5);
        assert!(
            r.iter().any(|c| c.name.starts_with("Córdoba") || c.name.starts_with("Cordoba")),
            "cordoba debería matchear Córdoba: {:?}",
            r.iter().map(|c| c.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn prefijo_rankea_por_poblacion() {
        // "san" trae muchísimas; la primera debe ser de gran población.
        let r = search("san", 3);
        assert!(!r.is_empty());
        assert!(r[0].population > 100_000);
    }

    #[test]
    fn offset_historico_con_dst() {
        // Buenos Aires en invierno (sin DST) = UTC-3 = -180 min.
        let inv = NaiveDate::from_ymd_opt(1980, 7, 1).unwrap().and_hms_opt(12, 0, 0).unwrap();
        assert_eq!(offset_minutes_at("America/Argentina/Buenos_Aires", inv), Some(-180));
        // Madrid en verano = UTC+2 = +120 (CEST).
        let ver = NaiveDate::from_ymd_opt(2000, 7, 1).unwrap().and_hms_opt(12, 0, 0).unwrap();
        assert_eq!(offset_minutes_at("Europe/Madrid", ver), Some(120));
        // Madrid en invierno = UTC+1 = +60 (CET).
        let inv_mad = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap().and_hms_opt(12, 0, 0).unwrap();
        assert_eq!(offset_minutes_at("Europe/Madrid", inv_mad), Some(60));
    }
}
