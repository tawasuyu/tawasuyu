//! Pantallazo headless de `minga-explorer-llimphi` — el dashboard de
//! observabilidad del repo Minga (VCS semántico P2P).
//!
//! Siembra un repo sled **real** en `/tmp/minga-pantallazo-repo`: ocho
//! fuentes (Rust / Python / TypeScript / JavaScript / Go) parseadas con
//! tree-sitter al grafo CAS, sus α-hashes como claves del MST y
//! atestaciones Ed25519 de verdad (tres autores con seeds fijas que se
//! avalan entre sí — el mismo flujo que `minga ingest` + `minga sign`).
//! Después monta la **view real** de la app (menubar, header con el path
//! del repo, y los tres stat-cards: nodos AST · atestaciones · claves
//! MST, cada uno con sus recientes) leyendo el snapshot con el mismo
//! `load_snapshot` que usa el polling, más el menú contextual de
//! observación abierto (refrescar / cambiar tema).
//!
//! Todo es determinista: seeds fijas, hashes BLAKE3 del contenido y
//! `last_load_ms` pineado — nada depende de la hora actual.
//!
//! Pinta a una textura wgpu sin ventana y vuelca PNG (mismo patrón que
//! `agora-app/examples/pantallazo_agora.rs`).
//!
//! `cargo run -p minga-explorer-llimphi --example pantallazo_minga --release -- [out.png]`
#![allow(dead_code)]

// La app es un crate binario sin lib: incluimos su módulo real por
// `#[path]` para llamar exactamente la misma `view` que pinta la app.
#[path = "../src/main.rs"]
mod app;

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use llimphi_motion::Tween;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::{measure_text_node, mount, paint, App, Mounted, View};
use minga_core::alpha::hash_alpha_with;
use minga_core::parse::Dialect;
use minga_core::{Attestation, Keypair};
use minga_store::PersistentRepo;

use crate::app::{Explorer, Model, Msg};

const W: u32 = 1600;
const H: u32 = 1000;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Las ocho fuentes que ingerimos al repo demo — fragmentos creíbles de
/// la propia temática de la suite (sync de pares, acequias comunales,
/// dedup de bloques). Cada una entra por el parser tree-sitter real de
/// su dialecto, así los `kind` de los nodos recientes son los de verdad
/// (`source_file`, `function_item`, …).
const FUENTES: &[(Dialect, &str)] = &[
    (
        Dialect::Rust,
        r#"//! Anuncio periódico de raíces locales a los pares de la minga.

pub struct Anunciante {
    intervalo_ms: u64,
    max_lote: usize,
}

impl Anunciante {
    pub fn nuevo(intervalo_ms: u64) -> Self {
        Self { intervalo_ms, max_lote: 32 }
    }

    /// Próximo lote de raíces a anunciar por gossip.
    pub fn lote<'a>(&self, raices: &'a [[u8; 32]]) -> &'a [[u8; 32]] {
        let n = raices.len().min(self.max_lote);
        &raices[..n]
    }
}
"#,
    ),
    (
        Dialect::Rust,
        r#"use std::collections::BTreeMap;

/// Tabla de pares conocidos con su última latencia observada.
pub struct TablaPares {
    pares: BTreeMap<String, u32>,
}

impl TablaPares {
    pub fn observar(&mut self, did: &str, latencia_ms: u32) {
        self.pares.insert(did.to_string(), latencia_ms);
    }

    pub fn mejores(&self, k: usize) -> Vec<&str> {
        let mut v: Vec<_> = self.pares.iter().collect();
        v.sort_by_key(|(_, lat)| **lat);
        v.into_iter().take(k).map(|(d, _)| d.as_str()).collect()
    }
}
"#,
    ),
    (
        Dialect::Rust,
        r#"/// Marca los hashes alcanzables desde las raíces vivas (GC mark).
pub fn alcanzables(raices: &[u64], hijos: impl Fn(u64) -> Vec<u64>) -> Vec<u64> {
    let mut vivos = Vec::new();
    let mut pila: Vec<u64> = raices.to_vec();
    while let Some(h) = pila.pop() {
        if !vivos.contains(&h) {
            vivos.push(h);
            pila.extend(hijos(h));
        }
    }
    vivos
}
"#,
    ),
    (
        Dialect::Python,
        r#""""Reparto de turnos de riego para la acequia comunal."""

def repartir(turnos, familias):
    asignados = {}
    for i, familia in enumerate(familias):
        asignados[familia] = turnos[i % len(turnos)]
    return asignados


def proximo(asignados, familia):
    return asignados.get(familia, "sin turno")
"#,
    ),
    (
        Dialect::Python,
        r#"import hashlib

def huella(ruta):
    h = hashlib.blake2b(digest_size=32)
    with open(ruta, "rb") as f:
        for bloque in iter(lambda: f.read(65536), b""):
            h.update(bloque)
    return h.hexdigest()
"#,
    ),
    (
        Dialect::TypeScript,
        r#"export interface Par {
  did: string;
  latenciaMs: number;
}

export function ordenarPares(pares: Par[]): Par[] {
  return [...pares].sort((a, b) => a.latenciaMs - b.latenciaMs);
}
"#,
    ),
    (
        Dialect::JavaScript,
        r#"export function trocear(buffer, tam = 4096) {
  const trozos = [];
  for (let i = 0; i < buffer.length; i += tam) {
    trozos.push(buffer.slice(i, i + tam));
  }
  return trozos;
}
"#,
    ),
    (
        Dialect::Go,
        r#"package almacen

import "bytes"

// Dedup quita bloques idénticos preservando el orden de llegada.
func Dedup(bloques [][]byte) [][]byte {
	var unicos [][]byte
	for _, b := range bloques {
		repetido := false
		for _, u := range unicos {
			if bytes.Equal(u, b) {
				repetido = true
				break
			}
		}
		if !repetido {
			unicos = append(unicos, b)
		}
	}
	return unicos
}
"#,
    ),
];

/// Siembra el repo sled en `<repo_path>/repo` con el mismo flujo que
/// `minga ingest`: parse → `nodes.put` (desempaqueta el árbol al CAS) →
/// α-hash → `roots.put` + `mst.insert` → atestación del autor. Encima,
/// cada raíz recibe el aval de un segundo autor (flujo `minga sign`).
/// Borra el repo previo para que el pantallazo sea reproducible.
fn sembrar_repo(repo_path: &Path) {
    let _ = std::fs::remove_dir_all(repo_path);
    std::fs::create_dir_all(repo_path).expect("crear dir del repo demo");
    let repo = PersistentRepo::open(repo_path.join("repo")).expect("abrir repo sled demo");

    // Tres autores con seeds fijas (32 bytes exactos) → DIDs estables.
    let autores = [
        Keypair::from_seed(b"amaru-semilla-minga-pantallazo-1"),
        Keypair::from_seed(b"killa-semilla-minga-pantallazo-2"),
        Keypair::from_seed(b"inti--semilla-minga-pantallazo-3"),
    ];

    for (i, (dialect, source)) in FUENTES.iter().enumerate() {
        let node = dialect.parse(source).expect("parse tree-sitter demo");
        let struct_hash = repo.nodes.put(&node).expect("put del árbol al CAS");
        let alpha = hash_alpha_with(*dialect, &node);
        repo.roots
            .put(alpha, struct_hash, *dialect)
            .expect("registrar raíz");
        repo.mst.insert(alpha).expect("clave al MST");

        // El autor que ingiere firma, y el siguiente lo avala (vouching).
        let autor = &autores[i % autores.len()];
        let aval = &autores[(i + 1) % autores.len()];
        repo.attestations
            .add(Attestation::create(autor, alpha))
            .expect("atestación del autor");
        repo.attestations
            .add(Attestation::create(aval, alpha))
            .expect("atestación del aval");
    }

    repo.flush().expect("flush del repo demo");
    // El drop suelta el lock de sled antes de que `load_snapshot` reabra.
}

/// Construye el `Model` demo: el mismo estado que tendría la app tras el
/// primer refresh exitoso, con el menú contextual de observación abierto
/// (como si el usuario acabara de hacer right-click sobre el dashboard).
fn modelo_demo(repo_path: PathBuf) -> Model {
    let snapshot = app::load_snapshot(&repo_path).expect("snapshot del repo sembrado");
    Model {
        theme: Theme::dark(),
        repo_path,
        snapshot: Some(snapshot),
        error: None,
        // Pineado: el header muestra "reload N ms" y no queremos que el
        // pantallazo dependa del reloj ni de la velocidad del disco.
        last_load_ms: 3,
        _wawa_watcher: None,
        menu_open: None,
        menu_active: usize::MAX,
        menu_anim: Tween::idle(1.0),
        // Ancla dentro del viewport lógico de la app (800×560): el
        // contextual clampea contra ese tamaño (ver `viewport_of`).
        context_menu: Some((540.0, 330.0)),
        toasts: Vec::new(),
        next_toast: 0,
        ticking: false,
    }
}

/// mount → layout → paint de un árbol de views a la `Scene` compartida —
/// la misma secuencia que el eventloop usa para la view y el overlay.
fn pintar_arbol(scene: &mut vello::Scene, ts: &mut Typesetter, view: View<Msg>) {
    let mut layout = LayoutTree::new();
    let mounted: Mounted<Msg> = mount(&mut layout, view);
    let computed = {
        let tmap = &mounted.text_measures;
        layout
            .compute_with_measure(mounted.root, (W as f32, H as f32), |nid, known, avail| {
                match tmap.get(&nid) {
                    Some(tm) => measure_text_node(ts, tm, known, avail),
                    None => taffy::Size::ZERO,
                }
            })
            .expect("layout")
    };
    paint(scene, &mounted, &computed, ts, None, None);
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/minga.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }

    // Locale fijo en español para que las cadenas del pantallazo no
    // dependan de la config de la máquina.
    rimay_localize::init();
    let _ = rimay_localize::set_locale("es");

    let repo_path = PathBuf::from("/tmp/minga-pantallazo-repo");
    sembrar_repo(&repo_path);
    let model = modelo_demo(repo_path);

    // view real + overlay real (contextual abierto), como el eventloop:
    // árboles de layout separados, pintados en orden a la misma escena.
    let mut ts = Typesetter::new();
    let mut scene = vello::Scene::new();
    pintar_arbol(&mut scene, &mut ts, Explorer::view(&model));
    if let Some(overlay) = Explorer::view_overlay(&model) {
        pintar_arbol(&mut scene, &mut ts, overlay);
    }

    let theme = &model.theme;
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-minga"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let [r, g, b, _] = theme.bg_app.components;
    let bg = Color::from_rgba8((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255);
    renderer
        .render_to_view(&hal, &scene, &view, W, H, bg)
        .expect("render_to_view");

    write_png(&hal, &target, &out);
    eprintln!("pantallazo_minga: escrito {out} ({W}x{H})");
}

/// Lee la textura a CPU y la vuelca como PNG RGBA8.
fn write_png(hal: &Hal, target: &wgpu::Texture, path: &str) {
    let unpadded = (W * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * H as usize) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    hal.queue.submit(std::iter::once(enc.finish()));
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((W * H * 4) as usize);
    for row in 0..H as usize {
        let s = row * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    w.write_image_data(&pixels).unwrap();
}
