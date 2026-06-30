//! Demo **headless** de la espina del file manager de Mónadas — sin levantar
//! la UI: monta un grafo real, lo edita, lo recorre por el trait `Source` y
//! muestra el despacho a nivel de Mónada. Todo a texto (la regla del repo:
//! certificar con stats impresas, no con PNG).
//!
//! Ejercita, en orden, las cuatro fases:
//!   0. modelo  — Mónadas con sub-Mónadas (DAG) y cuerpo intensional (query).
//!   1. Source  — navegación del grafo como un árbol de nodos.
//!   2. edit    — submonadizar una selección, anidar un álbum, crear una
//!                Mónada-consulta.
//!   3a. dispatch — lente → panel in-canvas + app de edición.
//!
//! Correr:
//!   cargo run -p nahual-shell-llimphi --example grafo_monadas
//!
//! Son las mismas APIs que la Fase 3b va a cablear a gestos del canvas, así
//! que este demo también es su banco de pruebas.

use std::fs;

use app_bus::{default_entries, AppRegistry};
use nahual_source_core::{
    edit, resolve, scanner, FileId, Lens, MonadDb, MonadQuery, NouserSource, Source,
};

fn main() {
    // ----- un árbol de archivos de juguete en un tempdir -----
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    write(root, "viaje/playa.jpg", b"\xff\xd8\xff");
    write(root, "viaje/atardecer.jpg", b"\xff\xd8\xff");
    write(root, "viaje/mar.jpg", b"\xff\xd8\xff");
    write(root, "familia/mama.jpg", b"\xff\xd8\xff");
    write(root, "proyecto/main.rs", b"fn main() {}");
    write(root, "proyecto/lib.rs", b"pub fn x() {}");
    write(root, "notas/ideas.md", b"# Ideas");
    write(root, "notas/pendientes.md", b"- algo");

    let files = scanner::scan_directory(root, &scanner::ScanConfig::default()).expect("scan");
    println!("Escaneados {} archivos en {}\n", files.len(), root.display());

    // índice por sufijo de ruta, para armar las Mónadas a mano (legible)
    let id = |suffix: &str| -> FileId {
        files
            .iter()
            .find(|f| f.path.ends_with(suffix))
            .unwrap_or_else(|| panic!("falta {suffix}"))
            .id
    };

    // ----- Fase 2: construir el grafo editándolo -----
    let mut db = MonadDb::new();
    db.ingest_files(files.clone());

    // "Viaje" = álbum extensional con sus 3 fotos.
    let viaje = edit::create_monad(&mut db, "Viaje", Lens::Gallery);
    for f in ["viaje/playa.jpg", "viaje/atardecer.jpg", "viaje/mar.jpg"] {
        edit::add_member(&mut db, viaje, id(f)).unwrap();
    }

    // "Familia" = álbum con una foto.
    let familia = edit::create_monad(&mut db, "Familia", Lens::Gallery);
    edit::add_member(&mut db, familia, id("familia/mama.jpg")).unwrap();

    // "Fotos" CONTIENE a Viaje y Familia (sub-Mónadas) → DAG de contención.
    let fotos = edit::create_monad(&mut db, "Fotos", Lens::Gallery);
    edit::add_submonad(&mut db, fotos, viaje).unwrap();
    edit::add_submonad(&mut db, fotos, familia).unwrap();

    // "Proyecto" = Mónada de código.
    let proyecto = edit::create_monad(&mut db, "Proyecto", Lens::Code);
    for f in ["proyecto/main.rs", "proyecto/lib.rs"] {
        edit::add_member(&mut db, proyecto, id(f)).unwrap();
    }

    // "Notas" extensional de markdown, de la que vamos a SUBMONADIZAR.
    let notas = edit::create_monad(&mut db, "Notas", Lens::Markdown);
    for f in ["notas/ideas.md", "notas/pendientes.md"] {
        edit::add_member(&mut db, notas, id(f)).unwrap();
    }

    // "Imágenes" = Mónada INTENSIONAL: su cuerpo es una query (todo lo que
    // sea galería), no una lista. No la contiene nadie → es top-level.
    let imagenes = edit::create_monad(&mut db, "Imágenes (todo)", Lens::Gallery);
    edit::set_query(&mut db, imagenes, Some(MonadQuery::imagenes())).unwrap();

    // ----- Fase 2: submonadizar -----
    // Sacamos "pendientes.md" de Notas a una hija "Tareas".
    println!("Submonadizando 'pendientes.md' fuera de Notas → hija 'Tareas'…\n");
    let _tareas = edit::submonadize(&mut db, notas, "Tareas", &[id("notas/pendientes.md")], &[]).unwrap();

    // ----- Fase 1: navegar el grafo por el trait Source -----
    let src = NouserSource::from_db(root.display().to_string(), db);
    println!("== Árbol de Mónadas (vía trait Source) ==");
    let raiz = src.root();
    imprimir(&src, &raiz.id, 0);

    // ----- métrica: archivos transitivos de "Fotos" (baja por el DAG) -----
    let n = src.with_db(|db| resolve::transitive_files(db, fotos)).len();
    println!("\n'Fotos' alcanza {n} archivos transitivamente (Viaje 3 + Familia 1).");

    // ----- Fase 3a: despacho por lente -----
    let reg = AppRegistry::new(default_entries());
    println!("\n== Despacho a nivel de Mónada (lente → panel · abrir en) ==");
    src.with_db(|db| {
        for m in db.monads() {
            let lens = m.dominant_lens;
            let app = default_app(&reg, lens).unwrap_or("—");
            println!(
                "  {:<16} {:<10} → panel {:<8} · abrir en {}",
                m.label,
                lens_str(lens),
                panel_str(lens),
                app
            );
        }
    });
}

/// Recorre el árbol de una `Source` imprimiéndolo indentado.
fn imprimir(src: &NouserSource, id: &str, nivel: usize) {
    let hijos = match src.children(&id.to_string()) {
        Ok(h) => h,
        Err(_) => return,
    };
    for n in hijos {
        let sangria = "  ".repeat(nivel + 1);
        let marca = if n.is_container { "▸" } else { "·" };
        let tam = n.size.map(|s| format!("  ({s} B)")).unwrap_or_default();
        println!("{sangria}{marca} {}{tam}", n.name);
        if n.is_container {
            imprimir(src, &n.id, nivel + 1);
        }
    }
}

// --- helpers de presentación (espejan nahual-shell-llimphi::monad_dispatch,
//     que es bin-privado y no se puede importar desde un example) ---

fn default_app<'a>(reg: &'a AppRegistry, lens: Lens) -> Option<&'a str> {
    let id = match lens {
        Lens::Gallery => "tullpu",
        Lens::Code => "nada",
        Lens::Database => "nakui",
        Lens::Markdown => "pluma",
        Lens::Tree => "nada",
        Lens::Grid => return None,
    };
    reg.get(id).map(|e| e.id.as_str())
}

fn panel_str(lens: Lens) -> &'static str {
    match lens {
        Lens::Gallery => "Gallery",
        Lens::Code | Lens::Tree => "Files",
        Lens::Database => "Sheet",
        Lens::Markdown => "Reader",
        Lens::Grid => "Generic",
    }
}

fn lens_str(lens: Lens) -> &'static str {
    match lens {
        Lens::Gallery => "gallery",
        Lens::Code => "code",
        Lens::Database => "database",
        Lens::Markdown => "markdown",
        Lens::Tree => "tree",
        Lens::Grid => "grid",
    }
}

fn write(root: &std::path::Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
