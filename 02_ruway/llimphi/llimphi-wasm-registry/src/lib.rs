//! llimphi-wasm-registry — puente data-driven a registros públicos de apps WASM.
//!
//! El catálogo propio (`llimphi-wasm-core::Catalog`) resuelve el "qué apps hay"
//! sobre la malla soberana. Este crate abre la otra puerta: **conectarse a
//! registros ya existentes en internet** (wasmer/WAPM, un índice estático, lo
//! que sea REST+JSON) de forma agnóstica y configurable, para tener catálogos
//! reales con qué probar sin publicar nada primero.
//!
//! La apuesta es la misma que `shared/foreign-platform` (de donde se copia el
//! patrón): un **descriptor RON** describe el endpoint de listado y el mapeo
//! campo-de-dominio → path-JSON; sumar un registro compatible es escribir un
//! `.ron`, no Rust.
//!
//! ## Honestidad de alcance
//!
//! Esto trae **metadatos + una URL de descarga** por app, y `ingest` baja el
//! módulo y lo vuelve un [`CatalogEntry`] resoluble (lo hashea al CAS — de ahí
//! sale el bytecode-hash, igual que `--install`). Lo que **no** garantiza es
//! que el módulo *corra* como app Tier 3: la mayoría del WASM público sigue
//! otro ABI (WASI, component-model, wasm-bindgen/DOM), no el `wasm_view →
//! WireNode` del runner. Para *probar el pipeline de descubrimiento/descarga*
//! sirve cualquiera; para *ejecutar como UI Llimphi* sólo corren los módulos
//! compilados contra `llimphi-wasm-app-sdk`. La verificación de integridad y de
//! concesión del core sigue valiendo: un módulo bajado de la red se hashea y,
//! si pretende capacidades, su concesión debe estar firmada por una clave de tu
//! anillo.

mod json;

use llimphi_wasm_core::{bytecode_hash, hash_to_hex, CatalogEntry, DiskStore, Hash};
use serde::Deserialize;
use serde_json::Value;

/// Errores del puente a un registro remoto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// El descriptor RON no parsea.
    Descriptor(String),
    /// Falla de red en el GET.
    Network(String),
    /// La respuesta no es JSON válido.
    Parse(String),
    /// El JSON no tiene la forma que el descriptor espera.
    Mapping(String),
    /// No se pudo guardar el módulo bajado en el CAS.
    Store(String),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::Descriptor(e) => write!(f, "descriptor inválido: {e}"),
            RegistryError::Network(e) => write!(f, "red: {e}"),
            RegistryError::Parse(e) => write!(f, "respuesta no-JSON: {e}"),
            RegistryError::Mapping(e) => write!(f, "mapeo: {e}"),
            RegistryError::Store(e) => write!(f, "guardar en CAS: {e}"),
        }
    }
}

impl std::error::Error for RegistryError {}

pub type RegistryResult<T> = Result<T, RegistryError>;

// ===========================================================================
//  Descriptor — el "script de texto" deserializable desde RON
// ===========================================================================

/// La descripción de un registro REST de apps WASM. Se deserializa de un `.ron`.
#[derive(Debug, Clone, Deserialize)]
pub struct RegistryDescriptor {
    /// Nombre legible ("wasmer", "índice local").
    pub name: String,
    /// Familia de API (informativo).
    #[serde(default)]
    pub kind: String,
    /// Instancias base sugeridas (la primera se usa si no se da otra).
    #[serde(default)]
    pub default_instances: Vec<String>,
    /// El endpoint que lista apps.
    pub list: ListEndpoint,
}

/// Un endpoint que devuelve una **lista** de apps.
#[derive(Debug, Clone, Deserialize)]
pub struct ListEndpoint {
    /// Path relativo a la instancia. Admite placeholder `{query}` (URL-encode).
    pub path: String,
    /// Query string clave→valor; los valores admiten `{query}`.
    #[serde(default)]
    pub query: Vec<(String, String)>,
    /// Dónde vive el array en la respuesta. `""` ⇒ la raíz ES el array.
    #[serde(default)]
    pub list_path: String,
    /// Cómo mapear cada item del array a una [`RemoteApp`].
    pub fields: AppFields,
}

/// Mapeo campo-de-dominio → path-JSON. `id`, `name` y `wasm_url` son
/// obligatorios (sin la URL no hay nada que ingerir); el resto opcional.
#[derive(Debug, Clone, Deserialize)]
pub struct AppFields {
    pub id: String,
    pub name: String,
    /// Path a la URL de descarga del `.wasm`.
    pub wasm_url: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

impl RegistryDescriptor {
    /// Parsea un descriptor desde texto RON.
    pub fn from_ron(src: &str) -> RegistryResult<Self> {
        ron::from_str(src).map_err(|e| RegistryError::Descriptor(e.to_string()))
    }
}

/// Una app **descubierta** en un registro remoto: metadatos + URL de descarga.
/// Todavía no está en el CAS (no tiene bytecode-hash) — eso lo hace [`ingest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteApp {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: Option<String>,
    pub version: Option<String>,
    pub wasm_url: String,
}

impl RemoteApp {
    /// ¿La consulta (sin distinguir mayúsculas) aparece en id/nombre/desc?
    pub fn matches(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let q = query.to_lowercase();
        self.id.to_lowercase().contains(&q)
            || self.name.to_lowercase().contains(&q)
            || self.description.to_lowercase().contains(&q)
    }
}

// ===========================================================================
//  Transporte HTTP — abstracto para testear sin red
// ===========================================================================

/// Abstracción del fetch HTTP. La real es [`UreqFetch`]; los tests inyectan una
/// que devuelve fixtures, así el mapeo se valida en CI sin red (igual criterio
/// que `foreign-platform` y el resto de los núcleos del repo).
pub trait HttpFetch {
    /// GET de una URL completa → texto del cuerpo.
    fn get_text(&self, url: &str) -> RegistryResult<String>;
    /// GET de una URL completa → bytes crudos (para bajar el `.wasm`).
    fn get_bytes(&self, url: &str) -> RegistryResult<Vec<u8>>;
}

/// Fetch real sobre `ureq` (HTTP síncrono, sin tokio).
#[derive(Debug, Clone, Default)]
pub struct UreqFetch;

impl HttpFetch for UreqFetch {
    fn get_text(&self, url: &str) -> RegistryResult<String> {
        ureq::get(url)
            .call()
            .map_err(|e| RegistryError::Network(e.to_string()))?
            .into_string()
            .map_err(|e| RegistryError::Network(e.to_string()))
    }

    fn get_bytes(&self, url: &str) -> RegistryResult<Vec<u8>> {
        let resp = ureq::get(url)
            .call()
            .map_err(|e| RegistryError::Network(e.to_string()))?;
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut resp.into_reader(), &mut buf)
            .map_err(|e| RegistryError::Network(e.to_string()))?;
        Ok(buf)
    }
}

/// Fetch desde el **filesystem local**: un "registro" que es un directorio con
/// un JSON de listado y los `.wasm`. No es red — sirve para un catálogo de
/// prueba reproducible y offline, y para tests. `root` es la base; una URL
/// absoluta se lee tal cual, una relativa se une a `root`.
#[derive(Debug, Clone)]
pub struct LocalFetch {
    pub root: std::path::PathBuf,
}

impl LocalFetch {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn resolve(&self, url: &str) -> std::path::PathBuf {
        let url = url.strip_prefix("file://").unwrap_or(url);
        let p = std::path::Path::new(url);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.root.join(url)
        }
    }
}

impl HttpFetch for LocalFetch {
    fn get_text(&self, url: &str) -> RegistryResult<String> {
        std::fs::read_to_string(self.resolve(url)).map_err(|e| RegistryError::Network(e.to_string()))
    }
    fn get_bytes(&self, url: &str) -> RegistryResult<Vec<u8>> {
        std::fs::read(self.resolve(url)).map_err(|e| RegistryError::Network(e.to_string()))
    }
}

// ===========================================================================
//  RegistryProvider — interpreta el descriptor sobre una instancia concreta
// ===========================================================================

/// Un registro concreto: un descriptor + una instancia base + un fetcher.
/// Genérico sobre el fetcher para inyectar fixtures en tests.
#[derive(Debug, Clone)]
pub struct RegistryProvider<F: HttpFetch = UreqFetch> {
    descriptor: RegistryDescriptor,
    base: String,
    fetch: F,
}

impl RegistryProvider<UreqFetch> {
    /// Construye sobre la instancia `base` (red real). `base` vacío ⇒ primera
    /// `default_instances` del descriptor.
    pub fn new(descriptor: RegistryDescriptor, base: impl Into<String>) -> Self {
        Self::with_fetch(descriptor, base, UreqFetch)
    }
}

impl<F: HttpFetch> RegistryProvider<F> {
    /// Construye con un fetcher explícito (tests).
    pub fn with_fetch(descriptor: RegistryDescriptor, base: impl Into<String>, fetch: F) -> Self {
        let mut base = base.into();
        if base.is_empty() {
            base = descriptor
                .default_instances
                .first()
                .cloned()
                .unwrap_or_default();
        }
        let base = base.trim_end_matches('/').to_string();
        Self { descriptor, base, fetch }
    }

    fn fill(tpl: &str, query: &str) -> String {
        tpl.replace("{query}", &url_encode(query))
    }

    fn list_url(&self, query: &str) -> String {
        let ep = &self.descriptor.list;
        let mut url = format!("{}{}", self.base, Self::fill(&ep.path, query));
        if !ep.query.is_empty() {
            let qs: Vec<String> = ep
                .query
                .iter()
                .map(|(k, v)| format!("{}={}", url_encode(k), Self::fill(v, query)))
                .collect();
            url.push('?');
            url.push_str(&qs.join("&"));
        }
        url
    }

    /// Lista las apps del registro (filtrando por `query` server-side vía el
    /// placeholder, y devolviendo lo que el endpoint dé). Mapea cada item según
    /// el descriptor; un item al que le falten campos obligatorios se descarta
    /// (no tumba la página entera).
    pub fn list(&self, query: &str) -> RegistryResult<Vec<RemoteApp>> {
        let url = self.list_url(query);
        let body = self.fetch.get_text(&url)?;
        let value: Value =
            serde_json::from_str(&body).map_err(|e| RegistryError::Parse(e.to_string()))?;
        let ep = &self.descriptor.list;
        let arr = json::get_array(&value, &ep.list_path).ok_or_else(|| {
            RegistryError::Mapping(format!("no hay array en «{}»", ep.list_path))
        })?;
        Ok(arr.iter().filter_map(|item| map_app(item, &ep.fields)).collect())
    }

    /// Descarga el módulo de una [`RemoteApp`], lo guarda en `store` (lo hashea
    /// — content-addressing) y devuelve un [`CatalogEntry`] resoluble. App de
    /// sólo-UI: `declarados = 0`, sin concesión (una capacidad exigiría una
    /// concesión firmada aparte). Es el puente registro-remoto → catálogo local.
    pub fn ingest(&self, app: &RemoteApp, store: &DiskStore) -> RegistryResult<CatalogEntry> {
        let bytes = self.fetch.get_bytes(&app.wasm_url)?;
        let bytecode = store.put(&bytes).map_err(|e| RegistryError::Store(e.to_string()))?;
        Ok(CatalogEntry {
            id: app.id.clone(),
            name: app.name.clone(),
            description: app.description.clone(),
            category: app.category.clone(),
            bytecode,
            declarados: 0,
            concesion: None,
        })
    }
}

/// El bytecode-hash que tendría una app si se ingiriera (sin guardar). Útil
/// para mostrar/cotejar antes de descargar al CAS definitivo.
pub fn hash_de(bytes: &[u8]) -> Hash {
    bytecode_hash(bytes)
}

/// Conveniencia: el hex del hash de unos bytes ya descargados.
pub fn hash_hex_de(bytes: &[u8]) -> String {
    hash_to_hex(&bytecode_hash(bytes))
}

fn map_app(item: &Value, f: &AppFields) -> Option<RemoteApp> {
    let id = json::get_string(item, &f.id)?;
    let name = json::get_string(item, &f.name)?;
    let wasm_url = json::get_string(item, &f.wasm_url)?;
    Some(RemoteApp {
        id,
        name,
        description: f
            .description
            .as_deref()
            .and_then(|p| json::get_string(item, p))
            .unwrap_or_default(),
        category: f.category.as_deref().and_then(|p| json::get_string(item, p)),
        version: f.version.as_deref().and_then(|p| json::get_string(item, p)),
        wasm_url,
    })
}

/// URL-encode mínimo (gemelo del de `foreign-platform`): deja a-zA-Z0-9-_.~ y
/// percent-codea el resto. Suficiente para queries de búsqueda.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESCRIPTOR_RON: &str = r#"#![enable(implicit_some)]
    (
        name: "ejemplo",
        kind: "rest-generico",
        default_instances: ["https://apps.example"],
        list: (
            path: "/v1/apps",
            query: [("q", "{query}"), ("limit", "20")],
            list_path: "results",
            fields: (
                id: "slug",
                name: "title",
                wasm_url: "download.wasm",
                description: "summary",
                category: "tags.0",
            ),
        ),
    )"#;

    /// Una respuesta JSON con la forma que el descriptor espera.
    fn cuerpo_lista() -> String {
        r#"{
            "results": [
                {
                    "slug": "counter",
                    "title": "Contador",
                    "summary": "suma y resta",
                    "tags": ["demo", "ui"],
                    "download": { "wasm": "https://apps.example/blobs/counter.wasm" }
                },
                {
                    "slug": "roto",
                    "title": "Sin URL"
                }
            ]
        }"#
        .to_string()
    }

    /// Fetcher de fixtures: texto canónico para el listado; bytes arbitrarios
    /// (el counter real lo inyecta el test de ingest) para la descarga.
    struct MockFetch {
        body: String,
        wasm: Vec<u8>,
    }
    impl HttpFetch for MockFetch {
        fn get_text(&self, _url: &str) -> RegistryResult<String> {
            Ok(self.body.clone())
        }
        fn get_bytes(&self, _url: &str) -> RegistryResult<Vec<u8>> {
            Ok(self.wasm.clone())
        }
    }

    fn provider(wasm: Vec<u8>) -> RegistryProvider<MockFetch> {
        let desc = RegistryDescriptor::from_ron(DESCRIPTOR_RON).expect("descriptor parsea");
        RegistryProvider::with_fetch(
            desc,
            "",
            MockFetch {
                body: cuerpo_lista(),
                wasm,
            },
        )
    }

    #[test]
    fn descriptor_ron_parsea() {
        let d = RegistryDescriptor::from_ron(DESCRIPTOR_RON).unwrap();
        assert_eq!(d.name, "ejemplo");
        assert_eq!(d.list.path, "/v1/apps");
        assert_eq!(d.list.fields.id, "slug");
    }

    #[test]
    fn list_mapea_e_ignora_items_rotos() {
        let p = provider(vec![]);
        let apps = p.list("cont").unwrap();
        // El item sin URL se descarta; queda el counter.
        assert_eq!(apps.len(), 1);
        let a = &apps[0];
        assert_eq!(a.id, "counter");
        assert_eq!(a.name, "Contador");
        assert_eq!(a.description, "suma y resta");
        assert_eq!(a.category.as_deref(), Some("demo"));
        assert_eq!(a.wasm_url, "https://apps.example/blobs/counter.wasm");
    }

    #[test]
    fn url_se_arma_con_query_y_encode() {
        let p = provider(vec![]);
        let url = p.list_url("hola mundo");
        assert_eq!(url, "https://apps.example/v1/apps?q=hola%20mundo&limit=20");
    }

    #[test]
    fn matches_busca_en_metadatos() {
        let p = provider(vec![]);
        let a = &p.list("").unwrap()[0];
        assert!(a.matches("CONT"));
        assert!(a.matches("suma"));
        assert!(!a.matches("zzz"));
        assert!(a.matches("")); // vacío = todo
    }

    /// El counter Tier 3 real — el "módulo bajado del registro".
    const COUNTER_WASM: &[u8] =
        include_bytes!("../../llimphi-wasm-runner/assets/counter.wasm");

    #[test]
    fn ingest_baja_hashea_y_da_catalog_entry() {
        use llimphi_wasm_core::{bytecode_hash, Catalog};

        let p = provider(COUNTER_WASM.to_vec());
        let app = p.list("").unwrap().into_iter().next().unwrap();

        let dir = std::env::temp_dir()
            .join(format!("llimphi-wasm-registry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = DiskStore::open(&dir).unwrap();

        let entry = p.ingest(&app, &store).expect("ingerir");
        // El bytecode-hash es el del módulo bajado, content-addressed.
        assert_eq!(entry.bytecode, bytecode_hash(COUNTER_WASM));
        assert_eq!(entry.id, "counter");
        assert!(entry.concesion.is_none(), "sólo-UI por defecto");
        // Y el blob quedó en el CAS: armamos un catálogo y se resuelve por id.
        assert_eq!(store.get(&entry.bytecode).as_deref(), Some(COUNTER_WASM));
        let cat = Catalog::new(vec![entry]);
        let v = llimphi_wasm_core::resolve_from_catalog(
            &store,
            &llimphi_wasm_core::TrustRing::empty(),
            &cat,
            "counter",
        )
        .expect("resuelve lo ingerido");
        assert_eq!(v.permisos, 0);
        assert_eq!(v.wasm, COUNTER_WASM);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn registro_local_offline_lista_e_ingiere() {
        // Un "registro" que es un directorio con apps.json + el .wasm. Sin red:
        // el catálogo de prueba reproducible que pidió el usuario.
        let dir = std::env::temp_dir()
            .join(format!("llimphi-wasm-registry-local-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("counter.wasm"), COUNTER_WASM).unwrap();
        std::fs::write(
            dir.join("apps.json"),
            r#"{"results":[{"slug":"counter","title":"Contador","summary":"demo local","tags":["demo"],"download":{"wasm":"counter.wasm"}}]}"#,
        )
        .unwrap();

        // Descriptor local: path directo al JSON, sin query (no es una URL real).
        let local_ron = r#"#![enable(implicit_some)]
        (
            name: "local",
            list: (
                path: "/apps.json",
                list_path: "results",
                fields: (id: "slug", name: "title", wasm_url: "download.wasm",
                         description: "summary", category: "tags.0"),
            ),
        )"#;
        let desc = RegistryDescriptor::from_ron(local_ron).unwrap();
        let provider = RegistryProvider::with_fetch(desc, dir.to_str().unwrap(), LocalFetch::new(&dir));

        let apps = provider.list("").unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].wasm_url, "counter.wasm");

        let cas = dir.join("cas");
        let store = DiskStore::open(&cas).unwrap();
        let entry = provider.ingest(&apps[0], &store).unwrap();
        assert_eq!(entry.bytecode, llimphi_wasm_core::bytecode_hash(COUNTER_WASM));
        assert_eq!(store.get(&entry.bytecode).as_deref(), Some(COUNTER_WASM));

        std::fs::remove_dir_all(&dir).ok();
    }
}
