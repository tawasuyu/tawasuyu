//! `pluma-outline` — proyección jerárquica de un `Cuerpo` plano.
//!
//! El modelo de pluma es deliberadamente *plano*: un `Cuerpo` es una
//! `Vec<Uuid>` de átomos, y un encabezado markdown vive como un átomo cuyo
//! texto empieza con el prefijo `"# "` / `"## "` / … (lo inyecta
//! `pluma-md::import`). El párrafo que sigue a un título no "sabe" que le
//! pertenece.
//!
//! Este crate **no cambia ese modelo** — lo *proyecta*. Recorre el orden plano
//! y construye un árbol de [`Seccion`]es donde cada título es un lienzo que
//! contiene su contenido: los párrafos que vienen tras él y las subsecciones de
//! nivel mayor, hasta que aparece otro título de nivel igual o menor. Así un
//! `## Motivación` cuelga dentro de su `# Introducción`, y el `### Notebook`
//! cuelga dentro de la `## Motivación`.
//!
//! La proyección es pura y barata: se recalcula tras cada edición. La fuente de
//! verdad sigue siendo la lista plana — no hay estado que sincronizar, no se
//! toca persistencia ni hebras.
//!
//! Además expone [`escala_por_nivel`] / [`font_size_por_nivel`]: el mapa de
//! tamaños de fuente por nivel de encabezado (h1 > h2 > h3 > … > cuerpo) que la
//! UI usa para pintar cada lienzo con su jerarquía tipográfica.

#![forbid(unsafe_code)]

use uuid::Uuid;

/// Nivel máximo de encabezado que markdown reconoce (`######`). Más `#` que
/// esto se trata como párrafo, igual que pulldown-cmark.
pub const NIVEL_MAX: u8 = 6;

/// Un nodo del árbol proyectado: o un párrafo suelto, o una sección (título +
/// su contenido anidado).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nodo {
    /// Un párrafo de cuerpo (átomo sin prefijo de encabezado).
    Parrafo {
        /// Identidad del átomo en el `Cuerpo`.
        atom: Uuid,
    },
    /// Una sección: un encabezado y todo lo que cuelga de él.
    Seccion(Seccion),
}

impl Nodo {
    /// `true` si el nodo es una sección.
    pub fn es_seccion(&self) -> bool {
        matches!(self, Nodo::Seccion(_))
    }

    /// El átomo que representa este nodo: el párrafo, o el título de la sección.
    pub fn atom(&self) -> Uuid {
        match self {
            Nodo::Parrafo { atom } => *atom,
            Nodo::Seccion(s) => s.titulo_atom,
        }
    }
}

/// Una sección del documento: un encabezado que actúa como *lienzo* y contiene
/// su contenido (párrafos y subsecciones más profundas).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seccion {
    /// El átomo del encabezado (el que lleva el prefijo `# `).
    pub titulo_atom: Uuid,
    /// Nivel del encabezado, `1..=NIVEL_MAX`.
    pub nivel: u8,
    /// El título ya *sin* el prefijo `# ` ni espacios sobrantes. Lo que pinta
    /// la cabecera del lienzo.
    pub titulo: String,
    /// Contenido del lienzo: párrafos y subsecciones, en orden de documento.
    pub hijos: Vec<Nodo>,
}

/// El árbol completo proyectado desde un `Cuerpo`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outline {
    /// Nodos de nivel raíz, en orden de documento. Los párrafos o secciones que
    /// aparecen antes de cualquier título cuelgan acá directamente.
    pub raiz: Vec<Nodo>,
}

impl Outline {
    /// `true` si no hay ningún nodo.
    pub fn vacio(&self) -> bool {
        self.raiz.is_empty()
    }

    /// Cantidad total de nodos (párrafos + secciones) contando todos los
    /// niveles de anidamiento.
    pub fn total_nodos(&self) -> usize {
        fn cuenta(nodos: &[Nodo]) -> usize {
            nodos
                .iter()
                .map(|n| match n {
                    Nodo::Parrafo { .. } => 1,
                    Nodo::Seccion(s) => 1 + cuenta(&s.hijos),
                })
                .sum()
        }
        cuenta(&self.raiz)
    }
}

/// El nivel de un átomo según su texto: `0` = párrafo de cuerpo, `1..=NIVEL_MAX`
/// = encabezado. Devuelve también el título limpio (sin prefijo) por
/// conveniencia del builder.
///
/// Reconoce un encabezado solo si el texto empieza con entre 1 y `NIVEL_MAX`
/// signos `#` seguidos de un espacio — exactamente el formato que produce
/// `pluma-md::import`. Un `#sintagma` sin espacio, o `#######` (7+), es párrafo.
pub fn nivel_de(texto: &str) -> (u8, &str) {
    let t = texto.trim_start();
    let hashes = t.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 || hashes > NIVEL_MAX as usize {
        return (0, texto.trim());
    }
    let resto = &t[hashes..];
    // Debe haber un espacio separando los `#` del título; si no, es párrafo.
    match resto.strip_prefix(' ') {
        Some(titulo) => (hashes as u8, titulo.trim()),
        None => (0, texto.trim()),
    }
}

/// Item aplanado intermedio: un átomo con su nivel y título ya resueltos. El
/// árbol se arma por rangos sobre un `Vec<Item>`.
struct Item {
    atom: Uuid,
    nivel: u8,
    titulo: String,
}

/// Proyecta el orden plano de átomos en un árbol jerárquico. `texto_de` resuelve
/// el contenido de cada átomo (típicamente `|id| atoms[&id].content.as_str()`);
/// si un id no resuelve, se trata como párrafo vacío (no rompe la proyección).
pub fn proyectar<'a, F>(orden: &[Uuid], texto_de: F) -> Outline
where
    F: Fn(Uuid) -> Option<&'a str>,
{
    let items: Vec<Item> = orden
        .iter()
        .map(|&atom| {
            let (nivel, titulo) = match texto_de(atom) {
                Some(t) => nivel_de(t),
                None => (0, ""),
            };
            Item {
                atom,
                nivel,
                titulo: titulo.to_string(),
            }
        })
        .collect();

    Outline {
        raiz: construir(&items),
    }
}

/// Construye recursivamente los nodos de un rango. Un encabezado de nivel `L`
/// posee todos los items que le siguen hasta el próximo encabezado de nivel
/// `<= L` (otro título hermano o de un ancestro), recursión que arma sus hijos.
fn construir(items: &[Item]) -> Vec<Nodo> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < items.len() {
        let it = &items[i];
        if it.nivel == 0 {
            out.push(Nodo::Parrafo { atom: it.atom });
            i += 1;
        } else {
            let l = it.nivel;
            // Extensión de la sección: hasta el próximo encabezado de nivel <= l.
            let mut j = i + 1;
            while j < items.len() {
                let n = items[j].nivel;
                if n != 0 && n <= l {
                    break;
                }
                j += 1;
            }
            out.push(Nodo::Seccion(Seccion {
                titulo_atom: it.atom,
                nivel: l,
                titulo: it.titulo.clone(),
                hijos: construir(&items[i + 1..j]),
            }));
            i = j;
        }
    }
    out
}

/// Escala tipográfica de un nivel de encabezado relativa al tamaño base del
/// cuerpo. `0` (párrafo) y `6` quedan en `1.0`; cada nivel más alto crece.
/// Pensada para multiplicar contra el `font_size` base del multilienzo.
pub fn escala_por_nivel(nivel: u8) -> f32 {
    match nivel {
        1 => 2.0,
        2 => 1.6,
        3 => 1.3,
        4 => 1.15,
        5 => 1.05,
        _ => 1.0,
    }
}

/// Tamaño de fuente concreto para un nivel, dado el `base` del cuerpo.
pub fn font_size_por_nivel(nivel: u8, base: f32) -> f32 {
    base * escala_por_nivel(nivel)
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use std::collections::HashMap;

    // Helper: construye un mapa atom→texto y devuelve (orden, resolver-friendly).
    fn doc(textos: &[&str]) -> (Vec<Uuid>, HashMap<Uuid, String>) {
        let mut orden = Vec::new();
        let mut mapa = HashMap::new();
        for t in textos {
            let id = Uuid::new_v4();
            orden.push(id);
            mapa.insert(id, t.to_string());
        }
        (orden, mapa)
    }

    fn proy(orden: &[Uuid], mapa: &HashMap<Uuid, String>) -> Outline {
        proyectar(orden, |id| mapa.get(&id).map(|s| s.as_str()))
    }

    #[test]
    fn nivel_de_reconoce_encabezados_y_cuerpo() {
        assert_eq!(nivel_de("# Título"), (1, "Título"));
        assert_eq!(nivel_de("## Sub"), (2, "Sub"));
        assert_eq!(nivel_de("###### Hondo"), (6, "Hondo"));
        // 7 hashes → párrafo.
        assert_eq!(nivel_de("####### Demasiado").0, 0);
        // Sin espacio → párrafo (un hashtag, no un título).
        assert_eq!(nivel_de("#etiqueta").0, 0);
        // Párrafo normal.
        assert_eq!(nivel_de("Hola mundo"), (0, "Hola mundo"));
    }

    #[test]
    fn parrafos_sueltos_quedan_en_raiz() {
        let (orden, mapa) = doc(&["Uno.", "Dos.", "Tres."]);
        let o = proy(&orden, &mapa);
        assert_eq!(o.raiz.len(), 3);
        assert!(o.raiz.iter().all(|n| matches!(n, Nodo::Parrafo { .. })));
    }

    #[test]
    fn titulo_contiene_sus_parrafos() {
        let (orden, mapa) = doc(&["# Intro", "Primer párrafo.", "Segundo párrafo."]);
        let o = proy(&orden, &mapa);
        assert_eq!(o.raiz.len(), 1);
        let Nodo::Seccion(s) = &o.raiz[0] else {
            panic!("esperaba sección");
        };
        assert_eq!(s.nivel, 1);
        assert_eq!(s.titulo, "Intro");
        assert_eq!(s.hijos.len(), 2);
        assert!(s.hijos.iter().all(|n| matches!(n, Nodo::Parrafo { .. })));
    }

    #[test]
    fn anidamiento_de_tres_niveles() {
        let (orden, mapa) = doc(&[
            "# Introducción",
            "El proyecto unifica.",
            "## Motivación",
            "Hoy hay tres apps.",
            "### Notebook",
            "Es la más completa.",
        ]);
        let o = proy(&orden, &mapa);
        // raíz: [Seccion(h1)]
        assert_eq!(o.raiz.len(), 1);
        let Nodo::Seccion(h1) = &o.raiz[0] else {
            panic!()
        };
        assert_eq!(h1.titulo, "Introducción");
        // h1.hijos: [Parrafo, Seccion(h2)]
        assert_eq!(h1.hijos.len(), 2);
        assert!(matches!(h1.hijos[0], Nodo::Parrafo { .. }));
        let Nodo::Seccion(h2) = &h1.hijos[1] else {
            panic!("esperaba h2 dentro de h1")
        };
        assert_eq!(h2.nivel, 2);
        assert_eq!(h2.titulo, "Motivación");
        // h2.hijos: [Parrafo, Seccion(h3)]
        let Nodo::Seccion(h3) = &h2.hijos[1] else {
            panic!("esperaba h3 dentro de h2")
        };
        assert_eq!(h3.nivel, 3);
        assert_eq!(h3.titulo, "Notebook");
        assert_eq!(h3.hijos.len(), 1);
    }

    #[test]
    fn hermano_de_igual_nivel_cierra_la_seccion() {
        let (orden, mapa) = doc(&["# Uno", "a", "# Dos", "b"]);
        let o = proy(&orden, &mapa);
        assert_eq!(o.raiz.len(), 2);
        let (Nodo::Seccion(s1), Nodo::Seccion(s2)) = (&o.raiz[0], &o.raiz[1]) else {
            panic!("esperaba dos secciones hermanas")
        };
        assert_eq!(s1.titulo, "Uno");
        assert_eq!(s1.hijos.len(), 1); // solo "a"
        assert_eq!(s2.titulo, "Dos");
        assert_eq!(s2.hijos.len(), 1); // solo "b"
    }

    #[test]
    fn subseccion_mas_profunda_no_se_lleva_al_hermano_del_padre() {
        // # A / ## B / # C : C es hermano de A, no hijo de B.
        let (orden, mapa) = doc(&["# A", "## B", "# C"]);
        let o = proy(&orden, &mapa);
        assert_eq!(o.raiz.len(), 2);
        let Nodo::Seccion(a) = &o.raiz[0] else { panic!() };
        assert_eq!(a.hijos.len(), 1); // contiene B
        assert!(a.hijos[0].es_seccion());
        let Nodo::Seccion(c) = &o.raiz[1] else { panic!() };
        assert_eq!(c.titulo, "C");
        assert!(c.hijos.is_empty());
    }

    #[test]
    fn salto_de_nivel_h1_a_h3_anida_igual() {
        // Un h3 tras un h1 (sin h2 intermedio) sigue colgando del h1.
        let (orden, mapa) = doc(&["# A", "### Profundo", "x"]);
        let o = proy(&orden, &mapa);
        let Nodo::Seccion(a) = &o.raiz[0] else { panic!() };
        assert_eq!(a.hijos.len(), 1);
        let Nodo::Seccion(p) = &a.hijos[0] else { panic!() };
        assert_eq!(p.nivel, 3);
        assert_eq!(p.hijos.len(), 1);
    }

    #[test]
    fn parrafos_antes_del_primer_titulo_quedan_en_raiz() {
        let (orden, mapa) = doc(&["Preámbulo.", "# Título", "cuerpo"]);
        let o = proy(&orden, &mapa);
        assert_eq!(o.raiz.len(), 2);
        assert!(matches!(o.raiz[0], Nodo::Parrafo { .. }));
        assert!(o.raiz[1].es_seccion());
    }

    #[test]
    fn total_nodos_cuenta_todos_los_niveles() {
        let (orden, mapa) = doc(&["# A", "x", "## B", "y", "z"]);
        let o = proy(&orden, &mapa);
        // A, x, B, y, z = 5
        assert_eq!(o.total_nodos(), 5);
    }

    #[test]
    fn id_sin_resolver_es_parrafo_vacio_no_panica() {
        let huerfano = Uuid::new_v4();
        let o = proyectar(&[huerfano], |_| None);
        assert_eq!(o.raiz.len(), 1);
        assert!(matches!(o.raiz[0], Nodo::Parrafo { .. }));
    }

    #[test]
    fn outline_vacio() {
        let o = proyectar(&[], |_| None);
        assert!(o.vacio());
        assert_eq!(o.total_nodos(), 0);
    }

    #[test]
    fn escala_decrece_con_el_nivel() {
        assert!(escala_por_nivel(1) > escala_por_nivel(2));
        assert!(escala_por_nivel(2) > escala_por_nivel(3));
        assert_eq!(escala_por_nivel(0), 1.0);
        assert_eq!(escala_por_nivel(6), 1.0);
        assert_eq!(font_size_por_nivel(1, 13.0), 26.0);
        assert_eq!(font_size_por_nivel(0, 13.0), 13.0);
    }
}
