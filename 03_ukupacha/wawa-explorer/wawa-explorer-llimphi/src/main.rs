//! `wawa-explorer-llimphi` — visor Llimphi de imágenes Wawa.
//!
//! Uso:
//!   wawa-explorer-llimphi <ruta.img> [iface]
//!
//! Renderea el grafo direccionado por contenido del disco Wawa: tree a la
//! izquierda con expand/collapse y selección, panel de detalle a la derecha
//! con header (hash + tamaño + aridad), hex preview de los primeros bytes
//! del payload y listado de hijos.
//!
//! Cuando un nodo está REFERENCIADO pero AUSENTE de la imagen local, el
//! panel ofrece un botón "fetch from peers" que pide el objeto por AoE a
//! la red local (`wawa-explorer-aoe`). El payload llega verificado
//! (`blake3(payload) == id`) y queda en memoria para la sesión actual —
//! la imagen original NO se modifica.
//!
//! Interfaz de red: pasada como segundo argumento, o auto-detectada
//! leyendo `/sys/class/net/` (la primera no-loopback con `operstate=up` y
//! MAC no cero). El cliente AoE necesita `CAP_NET_RAW` o root; sin esos
//! permisos el fetch falla con un mensaje legible.

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use format::{Hash, Objeto};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_tree::{tree_view, TreePalette, TreeRow, TreeSpec};
use wawa_explorer_aoe::ClienteAoE;
use wawa_explorer_core::{short_hex, Disco};

/// Timeout del fetch AoE: la red local responde en milisegundos; 3 s deja
/// margen para una retransmisión perdida sin colgar la UI.
const TIMEOUT_FETCH: Duration = Duration::from_secs(3);

#[derive(Clone)]
enum Msg {
    Toggle(Hash),
    Select(Hash),
    FetchPeers(Hash),
    FetchOk(Hash, Objeto),
    FetchFailed(Hash, String),
}

struct Model {
    disco: Option<Disco>,
    source: PathBuf,
    error: Option<String>,
    expanded: HashSet<Hash>,
    selected: Option<Hash>,
    raices: Vec<Hash>,
    /// Interfaz que usará el cliente AoE. `Err` lleva el motivo legible —
    /// se muestra en lugar del botón de fetch.
    iface: Result<String, String>,
    /// Objetos traídos por AoE — viven sólo en esta sesión.
    fetched: HashMap<Hash, Objeto>,
    /// Hashes con fetch en vuelo.
    fetching: HashSet<Hash>,
    /// Último error de fetch por hash. Se limpia cuando arranca un retry.
    fetch_errors: HashMap<Hash, String>,
}

struct Explorer;

impl App for Explorer {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "wawa-explorer"
    }

    fn initial_size() -> (u32, u32) {
        (1100, 720)
    }

    fn init(_: &Handle<Msg>) -> Model {
        let mut args = env::args().skip(1);
        let source = args.next().map(PathBuf::from).unwrap_or_else(|| PathBuf::from(""));
        let iface_arg = args.next();

        let iface = resolver_iface(iface_arg.as_deref());

        if source.as_os_str().is_empty() {
            return Model {
                disco: None,
                source,
                error: Some("uso: wawa-explorer-llimphi <ruta.img> [iface]".into()),
                expanded: HashSet::new(),
                selected: None,
                raices: Vec::new(),
                iface,
                fetched: HashMap::new(),
                fetching: HashSet::new(),
                fetch_errors: HashMap::new(),
            };
        }
        match Disco::abrir(&source) {
            Ok(d) => {
                let raices = raices_de(&d);
                let selected = raices.first().copied();
                Model {
                    disco: Some(d),
                    source,
                    error: None,
                    expanded: HashSet::new(),
                    selected,
                    raices,
                    iface,
                    fetched: HashMap::new(),
                    fetching: HashSet::new(),
                    fetch_errors: HashMap::new(),
                }
            }
            Err(e) => Model {
                disco: None,
                source,
                error: Some(e.to_string()),
                expanded: HashSet::new(),
                selected: None,
                raices: Vec::new(),
                iface,
                fetched: HashMap::new(),
                fetching: HashSet::new(),
                fetch_errors: HashMap::new(),
            },
        }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Toggle(h) => {
                if !model.expanded.remove(&h) {
                    model.expanded.insert(h);
                }
            }
            Msg::Select(h) => {
                model.selected = Some(h);
            }
            Msg::FetchPeers(h) => {
                let Ok(iface) = model.iface.clone() else {
                    return model;
                };
                if model.fetching.contains(&h) {
                    return model;
                }
                model.fetch_errors.remove(&h);
                model.fetching.insert(h);
                handle.spawn(move || pedir_objeto(&iface, h));
            }
            Msg::FetchOk(h, obj) => {
                model.fetching.remove(&h);
                model.fetched.insert(h, obj);
            }
            Msg::FetchFailed(h, e) => {
                model.fetching.remove(&h);
                model.fetch_errors.insert(h, e);
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let palette = Palette::from_theme(&theme);

        let header = header_view(model, &palette);
        let main = main_view(model, &theme, &palette);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(palette.bg)
        .children(vec![header, main])
    }
}

/// Determina las raíces a mostrar en el tree top-level. Prioridad:
/// manifest > raíz > orphans (objetos sin padre conocido). Si el disco
/// está vacío, lista vacía.
fn raices_de(d: &Disco) -> Vec<Hash> {
    let mut raices = Vec::new();
    if let Some(h) = d.superbloque().manifiesto {
        raices.push(h);
    }
    if let Some(h) = d.superbloque().raiz {
        if !raices.contains(&h) {
            raices.push(h);
        }
    }
    raices
}

/// Lookup unificado: primero el disco local, después los objetos
/// traídos por AoE en esta sesión.
fn lookup<'a>(model: &'a Model, hash: &Hash) -> Option<&'a Objeto> {
    if let Some(d) = &model.disco {
        if let Some(o) = d.objeto(hash) {
            return Some(o);
        }
    }
    model.fetched.get(hash)
}

/// Aplana el árbol a partir de las raíces, respetando el set de expandidos.
fn filas_visibles(model: &Model) -> Vec<TreeRow<Msg>> {
    if model.disco.is_none() {
        return Vec::new();
    }
    let mut rows = Vec::new();
    for &raiz in &model.raices {
        emitir_subtree(model, model.selected, raiz, 0, &mut rows);
    }
    rows
}

fn emitir_subtree(
    model: &Model,
    selected: Option<Hash>,
    hash: Hash,
    depth: usize,
    rows: &mut Vec<TreeRow<Msg>>,
) {
    let objeto = lookup(model, &hash);
    let hijos: &[Hash] = objeto.map(|o| o.hijos.as_slice()).unwrap_or(&[]);
    let has_children = !hijos.is_empty();
    let expanded_aqui = model.expanded.contains(&hash);

    let etiqueta = match objeto {
        Some(o) => {
            let marca = if model.fetched.contains_key(&hash) {
                rimay_localize::t("wawa-marker-via-aoe")
            } else {
                String::new()
            };
            format!(
                "{}  ·  {} bytes  ·  {} hijos{}",
                short_hex(&hash),
                o.datos.len(),
                o.hijos.len(),
                marca,
            )
        }
        None => {
            let estado = if model.fetching.contains(&hash) {
                rimay_localize::t("wawa-marker-searching")
            } else if model.fetch_errors.contains_key(&hash) {
                rimay_localize::t("wawa-marker-fetch-failed")
            } else {
                rimay_localize::t("wawa-marker-not-in-image")
            };
            format!("{}{}", short_hex(&hash), estado)
        }
    };

    rows.push(TreeRow {
        label: etiqueta,
        depth,
        has_children,
        expanded: expanded_aqui,
        selected: selected == Some(hash),
        on_toggle: Msg::Toggle(hash),
        on_select: Msg::Select(hash),
    });

    if expanded_aqui {
        for &h in hijos {
            emitir_subtree(model, selected, h, depth + 1, rows);
        }
    }
}

/// Paleta del explorer — slots semánticos sobre el Theme.
struct Palette {
    bg: Color,
    bg_panel: Color,
    fg_text: Color,
    fg_muted: Color,
    fg_error: Color,
}

impl Palette {
    fn from_theme(t: &Theme) -> Self {
        Self {
            bg: t.bg_app,
            bg_panel: t.bg_panel,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_error: t.fg_destructive,
        }
    }
}

use llimphi_ui::llimphi_raster::peniko::Color;

fn header_view(model: &Model, palette: &Palette) -> View<Msg> {
    let iface_chip = match &model.iface {
        Ok(name) => {
            rimay_localize::t_args("wawa-iface-ok", &[("name", name.as_str().into())])
        }
        Err(_) => rimay_localize::t("wawa-iface-err"),
    };
    let texto = match (&model.disco, &model.error) {
        (_, Some(e)) => rimay_localize::t_args(
            "wawa-header-error",
            &[("err", e.to_string().into())],
        ),
        (Some(d), None) => {
            let sb = d.superbloque();
            rimay_localize::t_args(
                "wawa-header",
                &[
                    ("source", model.source.display().to_string().into()),
                    ("bytes", d.bytes_imagen().to_string().into()),
                    ("version", sb.version.to_string().into()),
                    ("cursor", sb.cursor.to_string().into()),
                    ("objects", d.cantidad_objetos().to_string().into()),
                    ("iface", iface_chip.into()),
                ],
            )
        }
        (None, None) => "wawa-explorer".to_string(),
    };
    let color = if model.error.is_some() { palette.fg_error } else { palette.fg_muted };

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .text_aligned(texto, 11.0, color, Alignment::Start)
}

fn main_view(model: &Model, theme: &Theme, palette: &Palette) -> View<Msg> {
    let tree_palette = TreePalette::from_theme(theme);
    let rows = filas_visibles(model);
    let tree = tree_view(TreeSpec { rows, row_height: 22.0, indent_px: 16.0, palette: tree_palette });

    let tree_panel = View::new(Style {
        size: Size { width: length(420.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .clip(true)
    .children(vec![tree]);

    let detail = detail_view(model, theme, palette);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![tree_panel, detail])
}

fn detail_view(model: &Model, theme: &Theme, palette: &Palette) -> View<Msg> {
    let Some(hash) = model.selected else {
        return detail_chrome(
            &rimay_localize::t("wawa-detail-empty"),
            String::new(),
            None,
            palette,
        );
    };
    let Some(_) = &model.disco else {
        return detail_chrome("", String::new(), None, palette);
    };

    if let Some(obj) = lookup(model, &hash) {
        let origen = if model.fetched.contains_key(&hash) {
            rimay_localize::t("wawa-marker-via-aoe")
        } else {
            String::new()
        };
        let titulo = rimay_localize::t_args(
            "wawa-detail-title",
            &[
                ("hash", hex_completo(&hash).into()),
                ("bytes", obj.datos.len().to_string().into()),
                ("children", obj.hijos.len().to_string().into()),
                ("origen", origen.into()),
            ],
        );
        let mut cuerpo = String::new();
        cuerpo.push_str(&rimay_localize::t("wawa-detail-payload-header"));
        cuerpo.push_str("\n\n");
        cuerpo.push_str(&hex_dump(&obj.datos, 256));
        if !obj.hijos.is_empty() {
            cuerpo.push('\n');
            cuerpo.push_str(&rimay_localize::t("wawa-detail-children-header"));
            cuerpo.push('\n');
            let missing_mark = rimay_localize::t("wawa-detail-child-missing");
            for (i, h) in obj.hijos.iter().enumerate() {
                let mark = if lookup(model, h).is_some() { "" } else { missing_mark.as_str() };
                cuerpo.push_str(&format!("  {i:3}.  {}{}\n", short_hex(h), mark));
            }
        }
        return detail_chrome(&titulo, cuerpo, None, palette);
    }

    let titulo = rimay_localize::t_args(
        "wawa-detail-title-missing",
        &[("hash", hex_completo(&hash).into())],
    );
    let estado_action = if model.fetching.contains(&hash) {
        (
            format!(
                "{}\n\n{}",
                rimay_localize::t("wawa-detail-searching-aoe-1"),
                rimay_localize::t("wawa-detail-searching-aoe-2"),
            ),
            None,
        )
    } else if let Some(err) = model.fetch_errors.get(&hash) {
        let cuerpo = format!(
            "{}\n  {err}\n\n{}",
            rimay_localize::t("wawa-detail-fetch-error-1"),
            rimay_localize::t("wawa-detail-fetch-error-2"),
        );
        (
            cuerpo,
            Some((rimay_localize::t("wawa-btn-retry-fetch"), Msg::FetchPeers(hash))),
        )
    } else {
        match &model.iface {
            Ok(iface) => (
                format!(
                    "{}\n\n{}",
                    rimay_localize::t("wawa-detail-needs-fetch-1"),
                    rimay_localize::t_args(
                        "wawa-detail-needs-fetch-2",
                        &[("iface", iface.as_str().into())],
                    ),
                ),
                Some((rimay_localize::t("wawa-btn-fetch"), Msg::FetchPeers(hash))),
            ),
            Err(why) => (
                format!(
                    "{}\n\n{}\n\n{}",
                    rimay_localize::t("wawa-detail-aoe-disabled-1"),
                    rimay_localize::t_args(
                        "wawa-detail-aoe-disabled-2",
                        &[("why", why.to_string().into())],
                    ),
                    rimay_localize::t("wawa-detail-aoe-disabled-3"),
                ),
                None,
            ),
        }
    };
    let (cuerpo, action) = estado_action;
    detail_chrome(&titulo, cuerpo, action.map(|(l, m)| (l, m, theme)), palette)
}

fn detail_chrome(
    titulo: &str,
    cuerpo: String,
    action: Option<(String, Msg, &Theme)>,
    palette: &Palette,
) -> View<Msg> {
    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(titulo.to_string(), 11.0, palette.fg_text, Alignment::Start);

    let body = View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(6.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(cuerpo, 11.0, palette.fg_muted, Alignment::Start);

    let mut children = vec![header, body];
    if let Some((label, msg, theme)) = action {
        let btn_palette = ButtonPalette::from_theme(theme);
        let btn_row = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(48.0_f32) },
            padding: Rect {
                left: length(12.0_f32),
                right: length(12.0_f32),
                top: length(8.0_f32),
                bottom: length(8.0_f32),
            },
            ..Default::default()
        })
        .children(vec![button_view(label, &btn_palette, msg)]);
        children.push(btn_row);
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(palette.bg)
    .clip(true)
    .children(children)
}

fn hex_completo(h: &Hash) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

/// Hex dump tipo `xxd`: 16 bytes por línea, offset a la izquierda, hex en
/// el medio. Cap a `max_bytes` para mantener el render barato.
fn hex_dump(bytes: &[u8], max_bytes: usize) -> String {
    let n = bytes.len().min(max_bytes);
    let mut out = String::new();
    for chunk_idx in 0..n.div_ceil(16) {
        let start = chunk_idx * 16;
        let end = (start + 16).min(n);
        out.push_str(&format!("  {start:04x}  "));
        for b in &bytes[start..end] {
            out.push_str(&format!("{b:02x} "));
        }
        out.push('\n');
    }
    if bytes.len() > max_bytes {
        out.push_str(&format!("  … ({} bytes más)\n", bytes.len() - max_bytes));
    }
    out
}

// =============================================================================
//  Detección de interfaz default y fetch AoE en background
// =============================================================================

/// Resuelve la interfaz a usar para AoE. Si el caller pasó una explícita
/// la honra. Si no, lee `/sys/class/net/` y elige la primera no-loopback
/// con `operstate=up` y MAC distinta de cero. En cualquier fallo devuelve
/// `Err(motivo)` legible para mostrar en lugar del botón.
fn resolver_iface(explicita: Option<&str>) -> Result<String, String> {
    if let Some(name) = explicita {
        if name.is_empty() {
            return Err("interfaz vacía en CLI".into());
        }
        return Ok(name.to_string());
    }
    let entries = fs::read_dir("/sys/class/net")
        .map_err(|e| format!("no pude listar /sys/class/net: {e}"))?;
    let mut candidatas: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "lo" {
            continue;
        }
        let operstate = fs::read_to_string(format!("/sys/class/net/{name}/operstate"))
            .unwrap_or_default();
        let address = fs::read_to_string(format!("/sys/class/net/{name}/address"))
            .unwrap_or_default();
        if operstate.trim() == "up" && address.trim() != "00:00:00:00:00:00" {
            candidatas.push(name);
        }
    }
    candidatas.sort();
    candidatas
        .into_iter()
        .next()
        .ok_or_else(|| "no detecté ninguna interfaz no-loopback con operstate=up".into())
}

/// Ejecuta un ciclo completo de fetch AoE: abre cliente, broadcast pedido,
/// espera respuesta, deserializa payload a `Objeto`. Devuelve `FetchOk` o
/// `FetchFailed(motivo)`. Pensado para correr en un thread aparte vía
/// `Handle::spawn`.
fn pedir_objeto(iface: &str, hash: Hash) -> Msg {
    let cliente = match ClienteAoE::nuevo(iface) {
        Ok(c) => c,
        Err(e) => {
            return Msg::FetchFailed(hash, formatear_error_cliente(iface, e));
        }
    };
    match cliente.solicitar(hash, TIMEOUT_FETCH) {
        Ok(Some(payload)) => match Objeto::deserializar(&payload) {
            Ok(obj) => Msg::FetchOk(hash, obj),
            Err(_) => Msg::FetchFailed(
                hash,
                "peer respondió con bytes que no decodifican a Objeto (postcard inválido)"
                    .into(),
            ),
        },
        Ok(None) => Msg::FetchFailed(
            hash,
            format!("timeout: ningún peer respondió en {} s", TIMEOUT_FETCH.as_secs()),
        ),
        Err(e) => Msg::FetchFailed(hash, format!("error de socket: {e}")),
    }
}

/// Traduce el error técnico del cliente AoE en una frase corta accionable.
/// Caso típico: falta `CAP_NET_RAW` (EPERM al abrir socket).
fn formatear_error_cliente(iface: &str, e: wawa_explorer_aoe::Error) -> String {
    use wawa_explorer_aoe::Error as E;
    match e {
        E::Io(io) if io.raw_os_error() == Some(libc_eperm()) => {
            "permiso denegado al abrir raw socket. Ejecutá con sudo o aplicá \
             `sudo setcap cap_net_raw=eip <binario>`."
                .into()
        }
        E::InterfazInaccesible(_) => format!("interfaz `{iface}` no existe o no es accesible"),
        otro => otro.to_string(),
    }
}

/// EPERM como `i32` sin tirar de la dep `libc` desde aquí. El valor lo
/// fija POSIX y Linux lo respeta — 1 en todas las arquitecturas que nos
/// interesan.
const fn libc_eperm() -> i32 {
    1
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<Explorer>();
}
