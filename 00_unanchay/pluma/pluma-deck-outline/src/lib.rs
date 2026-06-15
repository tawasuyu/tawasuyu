//! `pluma-deck-outline` — adapta el **outline** jerárquico de un `Cuerpo` a un
//! [`Recorrido`] del deck espacial (tipo Prezi).
//!
//! El adaptador que ya vive en `pluma-deck-core` (feature `pluma`) agrupa
//! *plano*: un marco por encabezado markdown, sin jerarquía. Este crate trabaja
//! a otra escala — proyecta el outline con [`pluma_outline::proyectar`] y emite
//! **un marco por sección de nivel raíz**, aplanando todo su subárbol (párrafos
//! y subsecciones) dentro del marco. Así un documento largo se vuela por sus
//! capítulos, no por cada párrafo.
//!
//! - Cada `Sección` (a CUALQUIER profundidad) → un [`Marco`] con
//!   [`ContenidoMarco::Texto`]`{ titulo: Some(..), parrafos }`, donde `parrafos`
//!   son sólo sus párrafos *directos*. Las subsecciones NO se aplanan: cada una
//!   es su propio marco. Así el vuelo entra a `# Cap`, luego a sus `## Sub`,
//!   recursivamente (orden depth-first = orden de lectura).
//! - Los párrafos sueltos antes del primer título se agrupan en un marco inicial
//!   ([`ContenidoMarco::Etiqueta`] si es uno solo, si no `Texto{titulo:None,..}`).
//! - Los marcos se colocan avanzando en X y bajando + encogiéndose con la
//!   profundidad (las subsecciones quedan más chicas y más abajo, sugiriendo el
//!   anidamiento espacial); `pasos` los recorre depth-first.
//!
//! El modelo de pluma sigue plano y agnóstico: esto sólo *proyecta* y *coloca*.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use pluma_deck_core::{ContenidoMarco, Marco, MarcoId, Recorrido};
use pluma_deck_core::Rect;
use pluma_outline::{proyectar, Nodo};
use pluma_cuerpo::Cuerpo;
use uuid::Uuid;

/// Ancho de cada marco en coordenadas de mundo.
pub const ANCHO_MARCO: f64 = 520.0;
/// Alto de cada marco en coordenadas de mundo.
pub const ALTO_MARCO: f64 = 360.0;
/// Avance horizontal entre marcos consecutivos (ancho + separación).
pub const PASO_X: f64 = ANCHO_MARCO + 220.0;
/// Desplazamiento vertical del zig-zag para los marcos impares.
pub const SALTO_Y: f64 = 240.0;

/// Proyecta el outline de un `Cuerpo` y lo adapta a un [`Recorrido`]: un marco
/// por sección de nivel raíz, más un marco inicial para los párrafos sueltos que
/// preceden al primer título. `texto_de` resuelve cada `Uuid` del cuerpo a su
/// contenido (el cuerpo no conoce el grafo de átomos; lo resuelve el caller);
/// los ids que no resuelven se tratan como párrafo vacío y se omiten del texto.
///
/// Los marcos se posicionan en zig-zag; `pasos` los recorre en orden de
/// documento. Cada marco recto (`rot_rad == 0.0`).
pub fn recorrido_desde_cuerpo(cuerpo: &Cuerpo, texto_de: impl Fn(Uuid) -> Option<String>) -> Recorrido {
    // Resolvemos los textos a un mapa local: `proyectar` quiere `Fn -> Option<&str>`,
    // así que necesitamos un dueño estable de los strings durante la proyección.
    let mut mapa: HashMap<Uuid, String> = HashMap::new();
    for &id in &cuerpo.orden {
        if let Some(t) = texto_de(id) {
            mapa.insert(id, t);
        }
    }

    let outline = proyectar(&cuerpo.orden, |id| mapa.get(&id).map(|s| s.as_str()));

    // Primera pasada: armamos `(contenido, profundidad)` en orden depth-first.
    let mut frames: Vec<(ContenidoMarco, usize)> = Vec::new();
    let mut sueltos: Vec<String> = Vec::new();

    let volcar_sueltos = |sueltos: &mut Vec<String>, frames: &mut Vec<(ContenidoMarco, usize)>| {
        if sueltos.is_empty() {
            return;
        }
        let tomados = std::mem::take(sueltos);
        let c = if tomados.len() == 1 {
            ContenidoMarco::Etiqueta(tomados.into_iter().next().unwrap())
        } else {
            ContenidoMarco::Texto { titulo: None, parrafos: tomados }
        };
        frames.push((c, 0));
    };

    for nodo in &outline.raiz {
        match nodo {
            Nodo::Parrafo { atom } => {
                if let Some(t) = texto_limpio(&mapa, atom) {
                    sueltos.push(t);
                }
            }
            Nodo::Seccion(s) => {
                // Cerramos el bloque de párrafos sueltos que precedía a la sección.
                volcar_sueltos(&mut sueltos, &mut frames);
                emitir_seccion(s, 0, &mapa, &mut frames);
            }
        }
    }
    // Párrafos sueltos al final (documento sin ningún título).
    volcar_sueltos(&mut sueltos, &mut frames);

    // Segunda pasada: colocamos cada contenido en el plano y armamos la ruta.
    // X avanza por marco; Y baja con la profundidad y el tamaño se encoge — las
    // subsecciones quedan visiblemente "dentro/debajo" de su capítulo.
    let mut rec = Recorrido::new();
    for (i, (contenido, depth)) in frames.into_iter().enumerate() {
        let id = (i + 1) as MarcoId;
        let escala = 1.0 / (1.0 + 0.3 * depth as f64);
        let x = i as f64 * PASO_X;
        let y = depth as f64 * SALTO_Y;
        let marco = Marco::new(
            id,
            Rect::new(x, y, ANCHO_MARCO * escala, ALTO_MARCO * escala),
            contenido,
        );
        rec.agregar_marco(marco);
        rec.pasos.push(id);
    }
    rec
}

/// Emite el marco de una sección (título + sus párrafos directos) y luego, en
/// orden, los marcos de sus subsecciones (recursivo, depth-first). Las
/// subsecciones reciben `depth + 1`.
fn emitir_seccion(
    s: &pluma_outline::Seccion,
    depth: usize,
    mapa: &HashMap<Uuid, String>,
    frames: &mut Vec<(ContenidoMarco, usize)>,
) {
    let mut parrafos = Vec::new();
    for h in &s.hijos {
        if let Nodo::Parrafo { atom } = h {
            if let Some(t) = texto_limpio(mapa, atom) {
                parrafos.push(t);
            }
        }
    }
    frames.push((
        ContenidoMarco::Texto {
            titulo: Some(s.titulo.clone()),
            parrafos,
        },
        depth,
    ));
    for h in &s.hijos {
        if let Nodo::Seccion(sub) = h {
            emitir_seccion(sub, depth + 1, mapa, frames);
        }
    }
}

/// Texto de un átomo, limpiado; `None` si no resuelve o queda vacío.
fn texto_limpio(mapa: &HashMap<Uuid, String>, atom: &Uuid) -> Option<String> {
    let t = mapa.get(atom)?.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use pluma_cuerpo::Intencion;

    /// Construye un `Cuerpo` con los textos dados (un átomo por texto, en orden)
    /// y devuelve también el mapa id→texto para alimentar `recorrido_desde_cuerpo`.
    fn doc(textos: &[&str]) -> (Cuerpo, HashMap<Uuid, String>) {
        let mut c = Cuerpo::nuevo("es", "doc", Intencion::Original, 0);
        let mut mapa = HashMap::new();
        for t in textos {
            let id = Uuid::new_v4();
            c.agregar(id, 0);
            mapa.insert(id, t.to_string());
        }
        (c, mapa)
    }

    fn rec_de(c: &Cuerpo, mapa: &HashMap<Uuid, String>) -> Recorrido {
        recorrido_desde_cuerpo(c, |id| mapa.get(&id).cloned())
    }

    fn como_texto(c: &ContenidoMarco) -> (Option<&str>, &[String]) {
        match c {
            ContenidoMarco::Texto { titulo, parrafos } => (titulo.as_deref(), parrafos.as_slice()),
            otro => panic!("se esperaba ContenidoMarco::Texto, llegó {otro:?}"),
        }
    }

    #[test]
    fn doc_vacio_no_da_marcos() {
        let c = Cuerpo::nuevo("es", "vacio", Intencion::Original, 0);
        let mapa = HashMap::new();
        let rec = rec_de(&c, &mapa);
        assert!(rec.marcos.is_empty());
        assert!(rec.pasos.is_empty());
    }

    #[test]
    fn dos_secciones_top_level_dan_dos_marcos_y_dos_pasos() {
        let (c, mapa) = doc(&["# Uno", "a", "# Dos", "b"]);
        let rec = rec_de(&c, &mapa);
        assert_eq!(rec.marcos.len(), 2);
        assert_eq!(rec.pasos, vec![1, 2]);
        // Dos secciones de nivel raíz (depth 0): avanzan en X, ambas en y=0.
        assert!(rec.marcos[1].rect.x > rec.marcos[0].rect.x);
        assert_eq!(rec.marcos[0].rect.y, 0.0);
        assert_eq!(rec.marcos[1].rect.y, 0.0);
        assert_eq!(rec.marcos[0].rot_rad, 0.0);
        let (t0, _) = como_texto(&rec.marcos[0].contenido);
        let (t1, _) = como_texto(&rec.marcos[1].contenido);
        assert_eq!(t0, Some("Uno"));
        assert_eq!(t1, Some("Dos"));
    }

    #[test]
    fn parrafos_descendientes_van_en_el_marco_de_su_seccion() {
        let (c, mapa) = doc(&["# Intro", "Primer párrafo.", "Segundo párrafo."]);
        let rec = rec_de(&c, &mapa);
        assert_eq!(rec.marcos.len(), 1);
        let (titulo, parrafos) = como_texto(&rec.marcos[0].contenido);
        assert_eq!(titulo, Some("Intro"));
        assert_eq!(
            parrafos,
            &["Primer párrafo.".to_string(), "Segundo párrafo.".to_string()]
        );
    }

    #[test]
    fn subseccion_anidada_es_su_propio_marco_depth_first() {
        let (c, mapa) = doc(&[
            "# Introducción",
            "El proyecto unifica.",
            "## Motivación",
            "Hoy hay tres apps.",
        ]);
        let rec = rec_de(&c, &mapa);
        // Cada sección su propio marco: capítulo + subsección = 2 marcos.
        assert_eq!(rec.marcos.len(), 2);
        assert_eq!(rec.pasos, vec![1, 2]);
        let (t0, p0) = como_texto(&rec.marcos[0].contenido);
        assert_eq!(t0, Some("Introducción"));
        assert_eq!(p0, &["El proyecto unifica.".to_string()]); // sólo párrafos DIRECTOS
        let (t1, p1) = como_texto(&rec.marcos[1].contenido);
        assert_eq!(t1, Some("Motivación"));
        assert_eq!(p1, &["Hoy hay tres apps.".to_string()]);
        // La subsección está más abajo (depth 1) y es más chica que el capítulo.
        assert!(rec.marcos[1].rect.y > rec.marcos[0].rect.y);
        assert!(rec.marcos[1].rect.w < rec.marcos[0].rect.w);
    }

    #[test]
    fn parrafos_sueltos_antes_del_primer_titulo_forman_marco_inicial() {
        // Dos sueltos → marco Texto{titulo:None}; luego la sección.
        let (c, mapa) = doc(&["Preámbulo uno.", "Preámbulo dos.", "# Título", "cuerpo"]);
        let rec = rec_de(&c, &mapa);
        assert_eq!(rec.marcos.len(), 2);
        assert_eq!(rec.pasos, vec![1, 2]);
        let (t0, p0) = como_texto(&rec.marcos[0].contenido);
        assert_eq!(t0, None);
        assert_eq!(
            p0,
            &["Preámbulo uno.".to_string(), "Preámbulo dos.".to_string()]
        );
        let (t1, _) = como_texto(&rec.marcos[1].contenido);
        assert_eq!(t1, Some("Título"));
    }

    #[test]
    fn un_solo_parrafo_suelto_es_una_etiqueta() {
        let (c, mapa) = doc(&["Solo una línea de portada.", "# Cap", "x"]);
        let rec = rec_de(&c, &mapa);
        assert_eq!(rec.marcos.len(), 2);
        match &rec.marcos[0].contenido {
            ContenidoMarco::Etiqueta(s) => assert_eq!(s, "Solo una línea de portada."),
            otro => panic!("se esperaba Etiqueta, llegó {otro:?}"),
        }
    }
}
