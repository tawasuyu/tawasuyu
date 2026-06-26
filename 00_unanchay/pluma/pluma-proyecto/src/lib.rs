//! `pluma-proyecto` — el formato de archivo de proyecto de pluma (`.pluma`) con
//! **control de versiones**.
//!
//! Un proyecto es un **archivo único** que contiene un DAG *direccionado por
//! contenido* (BLAKE3 vía [`format::hash`] + postcard) de **snapshots** de
//! varios **documentos multilienzo**. Cada snapshot voluntario (un *push*,
//! estilo commit) congela el estado de todos los documentos del proyecto; los
//! commits encadenan a sus padres formando un DAG con **ramas** y **merge** —
//! no se versiona cada tecla, sólo los pushes deliberados.
//!
//! Capas:
//! - **objetos**: `Hash → bytes` content-addressed. Tres clases de objeto
//!   ([`DocEstado`], [`Arbol`], [`Commit`]) viven todas acá; el dedup es
//!   automático (mismo contenido ⇒ mismo hash).
//! - **refs**: `ramas` (`nombre → commit`) + `head` ([`Head`]).
//! - **trabajo**: la *working copy* mutable (los documentos como se editan
//!   ahora); no entra al DAG hasta el próximo [`Proyecto::push`].
//!
//! El crate no toca el reloj (recibe `timestamp` del caller, como
//! `pluma-cuerpo`) ni la UI: es núcleo puro y testeable.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use pluma_align::CartaHebras;
use pluma_core::NarrativeAtom;
use pluma_cuerpo::Cuerpo;
use pluma_estilo::EstiloLienzo;
use pluma_transform::Transformacion;

/// Identidad de un objeto: su hash BLAKE3 (reusa el tipo de `format`).
pub type Hash = format::Hash;
/// Identidad estable de un documento dentro del proyecto.
pub type DocId = Uuid;

/// Magia del archivo `.pluma`.
pub const MAGIA: [u8; 8] = *b"PLUMAPRY";
/// Versión del formato en disco.
pub const VERSION: u32 = 1;

/// Errores del proyecto.
#[derive(Debug)]
pub enum ProyectoError {
    Io(std::io::Error),
    Serde(&'static str),
    MagiaInvalida,
    VersionIncompatible(u32),
    ObjetoFaltante(Hash),
    RamaInexistente(String),
}

impl std::fmt::Display for ProyectoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProyectoError::Io(e) => write!(f, "io: {e}"),
            ProyectoError::Serde(s) => write!(f, "serde: {s}"),
            ProyectoError::MagiaInvalida => write!(f, "no es un archivo .pluma (magia inválida)"),
            ProyectoError::VersionIncompatible(v) => write!(f, "versión de proyecto {v} incompatible"),
            ProyectoError::ObjetoFaltante(h) => write!(f, "objeto faltante {}", hash_corto(h)),
            ProyectoError::RamaInexistente(n) => write!(f, "rama inexistente: {n}"),
        }
    }
}
impl std::error::Error for ProyectoError {}
impl From<std::io::Error> for ProyectoError {
    fn from(e: std::io::Error) -> Self {
        ProyectoError::Io(e)
    }
}

/// Hash en forma corta (7 hex) para UI/logs.
pub fn hash_corto(h: &Hash) -> String {
    let mut s = String::with_capacity(7);
    for b in &h[..4] {
        s.push_str(&format!("{b:02x}"));
    }
    s.truncate(7);
    s
}

/// Estado completo de un documento multilienzo (un haz) — la unidad que se
/// versiona. Es serde-serializable porque todos sus componentes lo son.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocEstado {
    pub nombre: String,
    pub cuerpos: Vec<Cuerpo>,
    pub atoms: Vec<NarrativeAtom>,
    pub cartas: Vec<CartaHebras>,
    pub transformaciones: Vec<Transformacion>,
    pub estilos: Vec<(Uuid, EstiloLienzo)>,
}

impl DocEstado {
    pub fn vacio(nombre: impl Into<String>) -> Self {
        Self {
            nombre: nombre.into(),
            cuerpos: Vec::new(),
            atoms: Vec::new(),
            cartas: Vec::new(),
            transformaciones: Vec::new(),
            estilos: Vec::new(),
        }
    }

    /// Puente desde las colecciones vivas de la app (átomos y estilos como
    /// `HashMap`) a un `DocEstado` versionable. Sólo conserva los átomos
    /// referenciados por algún cuerpo del haz y los estilos de esos cuerpos.
    pub fn desde_colecciones(
        nombre: impl Into<String>,
        cuerpos: &[Cuerpo],
        atoms: &std::collections::HashMap<Uuid, NarrativeAtom>,
        cartas: &[CartaHebras],
        transformaciones: &[Transformacion],
        estilos: &std::collections::HashMap<Uuid, EstiloLienzo>,
    ) -> Self {
        let ids_cuerpo: BTreeSet<Uuid> = cuerpos.iter().map(|c| c.id).collect();
        let ids_atom: BTreeSet<Uuid> =
            cuerpos.iter().flat_map(|c| c.orden.iter().copied()).collect();
        let mut atoms_v: Vec<NarrativeAtom> = ids_atom
            .iter()
            .filter_map(|id| atoms.get(id).cloned())
            .collect();
        atoms_v.sort_by_key(|a| a.id);
        let estilos_v: Vec<(Uuid, EstiloLienzo)> = estilos
            .iter()
            .filter(|(id, _)| ids_cuerpo.contains(id))
            .map(|(id, e)| (*id, e.clone()))
            .collect();
        Self {
            nombre: nombre.into(),
            cuerpos: cuerpos.to_vec(),
            atoms: atoms_v,
            cartas: cartas.to_vec(),
            transformaciones: transformaciones.to_vec(),
            estilos: estilos_v,
        }
    }

    /// Átomos como `HashMap` (la forma que usa la app en memoria).
    pub fn atoms_map(&self) -> std::collections::HashMap<Uuid, NarrativeAtom> {
        self.atoms.iter().map(|a| (a.id, a.clone())).collect()
    }

    /// Estilos como `HashMap`.
    pub fn estilos_map(&self) -> std::collections::HashMap<Uuid, EstiloLienzo> {
        self.estilos.iter().cloned().collect()
    }
}

/// Árbol de un snapshot: mapea cada documento a su hash de [`DocEstado`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Arbol {
    pub docs: BTreeMap<DocId, Hash>,
    /// Nombres de documento congelados en este árbol (para listarlos al
    /// previsualizar un commit sin cargar cada `DocEstado`).
    pub nombres: BTreeMap<DocId, String>,
}

/// Un commit: un snapshot del proyecto + sus padres en el DAG.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Commit {
    pub arbol: Hash,
    /// 0 padres = commit raíz; 1 = normal; ≥2 = merge.
    pub padres: Vec<Hash>,
    pub autor: String,
    pub timestamp: u64,
    pub mensaje: String,
}

/// A qué apunta HEAD: una rama (avanza al pushear) o un commit suelto
/// (*detached*, tras previsualizar/restaurar un commit viejo).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Head {
    Rama(String),
    Suelto(Hash),
}

/// Cómo cambió un átomo (o documento) entre dos versiones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaseDiff {
    Agregado,
    Eliminado,
    Modificado,
}

/// Un cambio de átomo en un diff: su id, la clase, y el texto relevante (el
/// nuevo para Agregado/Modificado, el viejo para Eliminado).
#[derive(Debug, Clone)]
pub struct CambioAtomDiff {
    pub id: Uuid,
    pub clase: ClaseDiff,
    pub texto: String,
}

/// Diff de un documento entre dos versiones. `doc_clase` = `Some` si el doc
/// entero se agregó/eliminó; `None` si se modificó (existe en ambas).
#[derive(Debug, Clone)]
pub struct DocDiff {
    pub doc: DocId,
    pub nombre: String,
    pub doc_clase: Option<ClaseDiff>,
    pub atomos: Vec<CambioAtomDiff>,
}

/// Diff entre dos commits: los documentos que cambiaron.
#[derive(Debug, Clone, Default)]
pub struct Diff {
    pub docs: Vec<DocDiff>,
}

/// Resultado de un merge.
#[derive(Debug, Clone, PartialEq)]
pub enum ResultadoMerge {
    /// La rama actual ya estaba al día (otra es ancestro).
    AlDia,
    /// Avance directo: la rama actual era ancestro de la otra.
    FastForward(Hash),
    /// Merge real (commit de 2 padres). `conflictos` = docs cambiados en ambos
    /// lados; se tomó la versión de la rama actual ("ours").
    Merge { commit: Hash, conflictos: Vec<DocId> },
}

/// El proyecto en memoria. Serde-serializable: ES la forma en disco (envuelta
/// por [`ArchivoProyecto`] con magia/versión).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proyecto {
    pub nombre: String,
    objetos: BTreeMap<Hash, Vec<u8>>,
    ramas: BTreeMap<String, Hash>,
    head: Head,
    trabajo: BTreeMap<DocId, DocEstado>,
}

/// Forma en disco del `.pluma`: magia + versión + proyecto.
#[derive(Serialize, Deserialize)]
struct ArchivoProyecto {
    magia: [u8; 8],
    version: u32,
    proyecto: Proyecto,
}

/// Rama por defecto de un proyecto nuevo.
pub const RAMA_DEFECTO: &str = "principal";

impl Proyecto {
    /// Crea un proyecto vacío: sin commits, una rama [`RAMA_DEFECTO`], HEAD en
    /// ella, working copy vacía.
    pub fn nuevo(nombre: impl Into<String>) -> Self {
        Self {
            nombre: nombre.into(),
            objetos: BTreeMap::new(),
            ramas: BTreeMap::new(), // la rama nace al primer push
            head: Head::Rama(RAMA_DEFECTO.to_string()),
            trabajo: BTreeMap::new(),
        }
    }

    // ----- Working copy --------------------------------------------------

    /// Documentos de la working copy: `(id, nombre)`, ordenados por nombre.
    pub fn documentos(&self) -> Vec<(DocId, String)> {
        let mut v: Vec<(DocId, String)> = self
            .trabajo
            .iter()
            .map(|(id, d)| (*id, d.nombre.clone()))
            .collect();
        v.sort_by(|a, b| a.1.cmp(&b.1));
        v
    }

    /// Estado de trabajo de un documento.
    pub fn documento(&self, id: DocId) -> Option<&DocEstado> {
        self.trabajo.get(&id)
    }

    /// Inserta/reemplaza el estado de trabajo de un documento.
    pub fn set_documento(&mut self, id: DocId, estado: DocEstado) {
        self.trabajo.insert(id, estado);
    }

    /// Crea un documento vacío en la working copy y devuelve su id.
    pub fn nuevo_documento(&mut self, nombre: impl Into<String>) -> DocId {
        let id = Uuid::new_v4();
        self.trabajo.insert(id, DocEstado::vacio(nombre));
        id
    }

    /// Elimina un documento de la working copy.
    pub fn eliminar_documento(&mut self, id: DocId) {
        self.trabajo.remove(&id);
    }

    /// Renombra un documento de la working copy.
    pub fn renombrar_documento(&mut self, id: DocId, nombre: impl Into<String>) {
        if let Some(d) = self.trabajo.get_mut(&id) {
            d.nombre = nombre.into();
        }
    }

    /// Borra una rama. No borra la rama actual (devuelve `false`).
    pub fn borrar_rama(&mut self, nombre: &str) -> bool {
        if self.rama_actual() == Some(nombre) {
            return false;
        }
        self.ramas.remove(nombre).is_some()
    }

    // ----- Refs ----------------------------------------------------------

    /// Nombre de la rama actual, si HEAD está sobre una rama.
    pub fn rama_actual(&self) -> Option<&str> {
        match &self.head {
            Head::Rama(n) => Some(n.as_str()),
            Head::Suelto(_) => None,
        }
    }

    pub fn head(&self) -> &Head {
        &self.head
    }

    /// Lista de ramas `(nombre, commit)`, ordenada por nombre.
    pub fn ramas(&self) -> Vec<(String, Hash)> {
        self.ramas.iter().map(|(n, h)| (n.clone(), *h)).collect()
    }

    /// Commit al que apunta HEAD (None si todavía no hubo ningún push).
    pub fn head_commit(&self) -> Option<Hash> {
        match &self.head {
            Head::Rama(n) => self.ramas.get(n).copied(),
            Head::Suelto(h) => Some(*h),
        }
    }

    // ----- Objetos content-addressed -------------------------------------

    fn put_obj<T: Serialize>(&mut self, obj: &T) -> Hash {
        let bytes = postcard::to_allocvec(obj).expect("postcard serializa");
        let h = format::hash(&bytes);
        self.objetos.entry(h).or_insert(bytes);
        h
    }

    fn get_obj<T: for<'de> Deserialize<'de>>(&self, h: &Hash) -> Result<T, ProyectoError> {
        let bytes = self
            .objetos
            .get(h)
            .ok_or(ProyectoError::ObjetoFaltante(*h))?;
        postcard::from_bytes(bytes).map_err(|_| ProyectoError::Serde("deserializar objeto"))
    }

    /// Lee un commit por hash.
    pub fn commit(&self, h: &Hash) -> Result<Commit, ProyectoError> {
        self.get_obj(h)
    }

    /// Lee el árbol de un commit.
    pub fn arbol_de(&self, commit: &Commit) -> Result<Arbol, ProyectoError> {
        self.get_obj(&commit.arbol)
    }

    /// Lee el estado de un documento de un árbol.
    pub fn doc_estado(&self, arbol: &Arbol, doc: DocId) -> Result<Option<DocEstado>, ProyectoError> {
        match arbol.docs.get(&doc) {
            Some(h) => Ok(Some(self.get_obj(h)?)),
            None => Ok(None),
        }
    }

    // ----- Push ----------------------------------------------------------

    /// Sella un snapshot voluntario de toda la working copy. Devuelve el hash
    /// del commit nuevo, o `None` si nada cambió respecto del HEAD (dedup por
    /// contenido: el árbol resultante es idéntico).
    pub fn push(
        &mut self,
        autor: impl Into<String>,
        mensaje: impl Into<String>,
        timestamp: u64,
    ) -> Option<Hash> {
        let arbol = self.snapshot_trabajo();
        let arbol_hash = self.put_obj(&arbol);

        // Dedup: si el HEAD ya apunta a un commit con este mismo árbol, no-op.
        let padre = self.head_commit();
        if let Some(p) = padre {
            if let Ok(c) = self.commit(&p) {
                if c.arbol == arbol_hash {
                    return None;
                }
            }
        }

        let commit = Commit {
            arbol: arbol_hash,
            padres: padre.into_iter().collect(),
            autor: autor.into(),
            timestamp,
            mensaje: mensaje.into(),
        };
        let commit_hash = self.put_obj(&commit);
        self.avanzar_head(commit_hash);
        Some(commit_hash)
    }

    /// Construye el [`Arbol`] de la working copy (hashea cada `DocEstado`).
    fn snapshot_trabajo(&mut self) -> Arbol {
        // Recogemos primero para no chocar con el borrow mutable de put_obj.
        let docs: Vec<(DocId, DocEstado)> =
            self.trabajo.iter().map(|(id, d)| (*id, d.clone())).collect();
        let mut arbol = Arbol::default();
        for (id, estado) in docs {
            arbol.nombres.insert(id, estado.nombre.clone());
            let h = self.put_obj(&estado);
            arbol.docs.insert(id, h);
        }
        arbol
    }

    /// Avanza la ref actual al commit dado (la rama si HEAD está sobre una; si
    /// no, mueve el HEAD suelto).
    fn avanzar_head(&mut self, commit: Hash) {
        match &self.head {
            Head::Rama(n) => {
                let n = n.clone();
                self.ramas.insert(n, commit);
            }
            Head::Suelto(_) => self.head = Head::Suelto(commit),
        }
    }

    // ----- Historia ------------------------------------------------------

    /// Todos los commits alcanzables desde todas las ramas y HEAD, en orden
    /// topológico (padres antes que hijos), con su hash.
    pub fn historia(&self) -> Vec<(Hash, Commit)> {
        let mut raices: Vec<Hash> = self.ramas.values().copied().collect();
        if let Some(h) = self.head_commit() {
            raices.push(h);
        }
        // DFS post-order para topo-sort (padres primero).
        let mut visto: BTreeSet<Hash> = BTreeSet::new();
        let mut orden: Vec<Hash> = Vec::new();
        for r in raices {
            self.topo(r, &mut visto, &mut orden);
        }
        orden
            .into_iter()
            .filter_map(|h| self.commit(&h).ok().map(|c| (h, c)))
            .collect()
    }

    fn topo(&self, h: Hash, visto: &mut BTreeSet<Hash>, orden: &mut Vec<Hash>) {
        if !visto.insert(h) {
            return;
        }
        if let Ok(c) = self.commit(&h) {
            for p in &c.padres {
                self.topo(*p, visto, orden);
            }
        }
        orden.push(h);
    }

    /// Conjunto de ancestros de un commit (incluido él).
    fn ancestros(&self, h: Hash) -> BTreeSet<Hash> {
        let mut set = BTreeSet::new();
        let mut pila = vec![h];
        while let Some(x) = pila.pop() {
            if !set.insert(x) {
                continue;
            }
            if let Ok(c) = self.commit(&x) {
                pila.extend(c.padres.iter().copied());
            }
        }
        set
    }

    // ----- Checkout / ramas ---------------------------------------------

    /// Carga el árbol de un commit en la working copy y deja HEAD *detached*
    /// sobre ese commit (previsualizar/restaurar una versión vieja). Un push
    /// posterior crea un commit nuevo encima sin perder la historia.
    pub fn checkout(&mut self, commit: Hash) -> Result<(), ProyectoError> {
        self.cargar_trabajo_de(commit)?;
        self.head = Head::Suelto(commit);
        Ok(())
    }

    fn cargar_trabajo_de(&mut self, commit: Hash) -> Result<(), ProyectoError> {
        let c = self.commit(&commit)?;
        let arbol = self.arbol_de(&c)?;
        let mut nuevo: BTreeMap<DocId, DocEstado> = BTreeMap::new();
        for (id, _) in arbol.docs.iter() {
            if let Some(d) = self.doc_estado(&arbol, *id)? {
                nuevo.insert(*id, d);
            }
        }
        self.trabajo = nuevo;
        Ok(())
    }

    /// Crea una rama `nombre` apuntando a `desde` (o al HEAD si `None`). No
    /// cambia el HEAD.
    pub fn rama_nueva(&mut self, nombre: impl Into<String>, desde: Option<Hash>) {
        let h = desde.or_else(|| self.head_commit());
        if let Some(h) = h {
            self.ramas.insert(nombre.into(), h);
        }
    }

    /// Cambia el HEAD a la rama `nombre` y carga su árbol en la working copy.
    pub fn cambiar_rama(&mut self, nombre: &str) -> Result<(), ProyectoError> {
        let commit = *self
            .ramas
            .get(nombre)
            .ok_or_else(|| ProyectoError::RamaInexistente(nombre.to_string()))?;
        self.cargar_trabajo_de(commit)?;
        self.head = Head::Rama(nombre.to_string());
        Ok(())
    }

    // ----- Merge ---------------------------------------------------------

    /// Mergea la rama `otra` en la rama actual. v1 a granularidad de
    /// **documento**: fast-forward cuando se puede; si no, árbol = unión por
    /// documento (cambiado en un lado → ese lado; en ambos → "ours" + se marca
    /// conflicto). Requiere HEAD sobre una rama.
    pub fn merge(
        &mut self,
        otra: &str,
        autor: impl Into<String>,
        timestamp: u64,
    ) -> Result<ResultadoMerge, ProyectoError> {
        let theirs = *self
            .ramas
            .get(otra)
            .ok_or_else(|| ProyectoError::RamaInexistente(otra.to_string()))?;
        let ours = match self.head_commit() {
            Some(h) => h,
            None => {
                // Rama actual vacía → FF directo a la otra.
                self.cargar_trabajo_de(theirs)?;
                self.avanzar_head(theirs);
                return Ok(ResultadoMerge::FastForward(theirs));
            }
        };

        let anc_ours = self.ancestros(ours);
        let anc_theirs = self.ancestros(theirs);

        if anc_ours.contains(&theirs) {
            return Ok(ResultadoMerge::AlDia); // ya tenemos lo suyo
        }
        if anc_theirs.contains(&ours) {
            // Fast-forward: nuestra rama es ancestro de la suya.
            self.cargar_trabajo_de(theirs)?;
            self.avanzar_head(theirs);
            return Ok(ResultadoMerge::FastForward(theirs));
        }

        // 3-way a nivel de documento.
        let base = self.merge_base(&anc_ours, theirs);
        let arbol_ours = self.arbol_de(&self.commit(&ours)?)?;
        let arbol_theirs = self.arbol_de(&self.commit(&theirs)?)?;
        let arbol_base = match base {
            Some(b) => self.arbol_de(&self.commit(&b)?)?,
            None => Arbol::default(),
        };

        let mut merged = Arbol::default();
        let mut conflictos: Vec<DocId> = Vec::new();
        let ids: BTreeSet<DocId> = arbol_ours
            .docs
            .keys()
            .chain(arbol_theirs.docs.keys())
            .copied()
            .collect();
        for id in ids {
            let o = arbol_ours.docs.get(&id).copied();
            let t = arbol_theirs.docs.get(&id).copied();
            let b = arbol_base.docs.get(&id).copied();
            let elegido = match (o, t) {
                (Some(o), Some(t)) if o == t => Some(o),
                (Some(o), Some(t)) => {
                    if Some(o) == b {
                        Some(t) // sólo cambió theirs
                    } else if Some(t) == b {
                        Some(o) // sólo cambió ours
                    } else {
                        conflictos.push(id);
                        Some(o) // ours en conflicto
                    }
                }
                (Some(o), None) => Some(o),
                (None, Some(t)) => Some(t),
                (None, None) => None,
            };
            if let Some(h) = elegido {
                merged.docs.insert(id, h);
                let nombre = arbol_ours
                    .nombres
                    .get(&id)
                    .or_else(|| arbol_theirs.nombres.get(&id))
                    .cloned()
                    .unwrap_or_default();
                merged.nombres.insert(id, nombre);
            }
        }

        let arbol_hash = self.put_obj(&merged);
        let commit = Commit {
            arbol: arbol_hash,
            padres: vec![ours, theirs],
            autor: autor.into(),
            timestamp,
            mensaje: format!("merge {otra}"),
        };
        let commit_hash = self.put_obj(&commit);
        self.avanzar_head(commit_hash);
        self.cargar_trabajo_de(commit_hash)?;
        Ok(ResultadoMerge::Merge { commit: commit_hash, conflictos })
    }

    /// Mejor ancestro común: el primer ancestro de `theirs` que también es
    /// ancestro de `ours` (en orden BFS desde theirs).
    fn merge_base(&self, anc_ours: &BTreeSet<Hash>, theirs: Hash) -> Option<Hash> {
        let mut visto = BTreeSet::new();
        let mut cola = std::collections::VecDeque::new();
        cola.push_back(theirs);
        while let Some(x) = cola.pop_front() {
            if !visto.insert(x) {
                continue;
            }
            if anc_ours.contains(&x) {
                return Some(x);
            }
            if let Ok(c) = self.commit(&x) {
                cola.extend(c.padres.iter().copied());
            }
        }
        None
    }

    // ----- Diff ----------------------------------------------------------

    /// Diff entre dos commits (`viejo` = `None` ⇒ contra el vacío). Devuelve, por
    /// documento que cambió, los átomos agregados/eliminados/modificados. Los
    /// documentos sin cambios (mismo hash de árbol) se omiten.
    pub fn diff(&self, viejo: Option<Hash>, nuevo: Hash) -> Result<Diff, ProyectoError> {
        let arbol_n = self.arbol_de(&self.commit(&nuevo)?)?;
        let arbol_v = match viejo {
            Some(v) => self.arbol_de(&self.commit(&v)?)?,
            None => Arbol::default(),
        };
        let ids: BTreeSet<DocId> = arbol_n
            .docs
            .keys()
            .chain(arbol_v.docs.keys())
            .copied()
            .collect();
        let mut docs: Vec<DocDiff> = Vec::new();
        for id in ids {
            let hn = arbol_n.docs.get(&id).copied();
            let hv = arbol_v.docs.get(&id).copied();
            if hn == hv {
                continue; // sin cambios
            }
            let nombre = arbol_n
                .nombres
                .get(&id)
                .or_else(|| arbol_v.nombres.get(&id))
                .cloned()
                .unwrap_or_default();
            match (hn, hv) {
                (Some(hn), None) => {
                    let de: DocEstado = self.get_obj(&hn)?;
                    let atomos = de
                        .atoms
                        .iter()
                        .map(|a| CambioAtomDiff {
                            id: a.id,
                            clase: ClaseDiff::Agregado,
                            texto: a.content.to_string(),
                        })
                        .collect();
                    docs.push(DocDiff { doc: id, nombre, doc_clase: Some(ClaseDiff::Agregado), atomos });
                }
                (None, Some(hv)) => {
                    let de: DocEstado = self.get_obj(&hv)?;
                    let atomos = de
                        .atoms
                        .iter()
                        .map(|a| CambioAtomDiff {
                            id: a.id,
                            clase: ClaseDiff::Eliminado,
                            texto: a.content.to_string(),
                        })
                        .collect();
                    docs.push(DocDiff { doc: id, nombre, doc_clase: Some(ClaseDiff::Eliminado), atomos });
                }
                (Some(hn), Some(hv)) => {
                    let den: DocEstado = self.get_obj(&hn)?;
                    let dev: DocEstado = self.get_obj(&hv)?;
                    let mapn: BTreeMap<Uuid, &NarrativeAtom> =
                        den.atoms.iter().map(|a| (a.id, a)).collect();
                    let mapv: BTreeMap<Uuid, &NarrativeAtom> =
                        dev.atoms.iter().map(|a| (a.id, a)).collect();
                    let aids: BTreeSet<Uuid> =
                        mapn.keys().chain(mapv.keys()).copied().collect();
                    let mut atomos = Vec::new();
                    for aid in aids {
                        match (mapn.get(&aid), mapv.get(&aid)) {
                            (Some(an), None) => atomos.push(CambioAtomDiff {
                                id: aid,
                                clase: ClaseDiff::Agregado,
                                texto: an.content.to_string(),
                            }),
                            (None, Some(av)) => atomos.push(CambioAtomDiff {
                                id: aid,
                                clase: ClaseDiff::Eliminado,
                                texto: av.content.to_string(),
                            }),
                            (Some(an), Some(av)) => {
                                if an.content_hash != av.content_hash {
                                    atomos.push(CambioAtomDiff {
                                        id: aid,
                                        clase: ClaseDiff::Modificado,
                                        texto: an.content.to_string(),
                                    });
                                }
                            }
                            (None, None) => {}
                        }
                    }
                    if !atomos.is_empty() {
                        docs.push(DocDiff { doc: id, nombre, doc_clase: None, atomos });
                    }
                }
                (None, None) => {}
            }
        }
        Ok(Diff { docs })
    }

    // ----- Archivo .pluma ------------------------------------------------

    /// Guarda el proyecto entero (objetos + refs + working copy) a un `.pluma`.
    pub fn guardar(&self, path: impl AsRef<Path>) -> Result<(), ProyectoError> {
        let archivo = ArchivoProyecto {
            magia: MAGIA,
            version: VERSION,
            proyecto: self.clone(),
        };
        let bytes =
            postcard::to_allocvec(&archivo).map_err(|_| ProyectoError::Serde("serializar archivo"))?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    /// Abre un `.pluma`.
    pub fn abrir(path: impl AsRef<Path>) -> Result<Proyecto, ProyectoError> {
        let bytes = std::fs::read(path)?;
        let archivo: ArchivoProyecto =
            postcard::from_bytes(&bytes).map_err(|_| ProyectoError::Serde("deserializar archivo"))?;
        if archivo.magia != MAGIA {
            return Err(ProyectoError::MagiaInvalida);
        }
        if archivo.version != VERSION {
            return Err(ProyectoError::VersionIncompatible(archivo.version));
        }
        Ok(archivo.proyecto)
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use pluma_cuerpo::Intencion;

    fn doc_con(nombre: &str, textos: &[&str]) -> DocEstado {
        let mut d = DocEstado::vacio(nombre);
        let mut c = Cuerpo::nuevo("es", nombre, Intencion::Original, 0);
        for t in textos {
            let a = NarrativeAtom::new(*t, "es");
            c.agregar(a.id, 0);
            d.atoms.push(a);
        }
        d.cuerpos.push(c);
        d
    }

    #[test]
    fn proyecto_nuevo_sin_commits() {
        let p = Proyecto::nuevo("demo");
        assert!(p.head_commit().is_none());
        assert!(p.historia().is_empty());
        assert_eq!(p.rama_actual(), Some(RAMA_DEFECTO));
    }

    #[test]
    fn push_crea_commit_y_avanza_rama() {
        let mut p = Proyecto::nuevo("demo");
        let id = p.nuevo_documento("cap1");
        p.set_documento(id, doc_con("cap1", &["Hola"]));
        let h = p.push("ana", "primer push", 100).expect("commit");
        assert_eq!(p.head_commit(), Some(h));
        assert_eq!(p.historia().len(), 1);
        let c = p.commit(&h).unwrap();
        assert!(c.padres.is_empty());
        assert_eq!(c.mensaje, "primer push");
    }

    #[test]
    fn push_sin_cambios_es_noop() {
        let mut p = Proyecto::nuevo("demo");
        let id = p.nuevo_documento("d");
        p.set_documento(id, doc_con("d", &["x"]));
        p.push("ana", "c1", 100).unwrap();
        // Sin tocar el trabajo: el segundo push deduplica.
        assert!(p.push("ana", "c2", 101).is_none());
        assert_eq!(p.historia().len(), 1);
    }

    #[test]
    fn cadena_de_commits_encadena_padres() {
        let mut p = Proyecto::nuevo("demo");
        let id = p.nuevo_documento("d");
        p.set_documento(id, doc_con("d", &["v1"]));
        let h1 = p.push("ana", "c1", 100).unwrap();
        p.set_documento(id, doc_con("d", &["v2"]));
        let h2 = p.push("ana", "c2", 101).unwrap();
        let c2 = p.commit(&h2).unwrap();
        assert_eq!(c2.padres, vec![h1]);
        assert_eq!(p.historia().len(), 2);
    }

    #[test]
    fn checkout_restaura_estado_viejo() {
        let mut p = Proyecto::nuevo("demo");
        let id = p.nuevo_documento("d");
        p.set_documento(id, doc_con("d", &["v1"]));
        let h1 = p.push("ana", "c1", 100).unwrap();
        p.set_documento(id, doc_con("d", &["v2"]));
        p.push("ana", "c2", 101).unwrap();
        // Volver a v1.
        p.checkout(h1).unwrap();
        let d = p.documento(id).unwrap();
        assert_eq!(*d.atoms[0].content, "v1".to_string());
        assert!(matches!(p.head(), Head::Suelto(_)));
    }

    #[test]
    fn rama_y_fast_forward_merge() {
        let mut p = Proyecto::nuevo("demo");
        let id = p.nuevo_documento("d");
        p.set_documento(id, doc_con("d", &["base"]));
        let h1 = p.push("ana", "base", 100).unwrap();
        // Rama nueva desde h1; avanzar en ella.
        p.rama_nueva("feat", Some(h1));
        p.cambiar_rama("feat").unwrap();
        p.set_documento(id, doc_con("d", &["feat"]));
        p.push("ana", "feat work", 101).unwrap();
        // Volver a principal y mergear feat → fast-forward.
        p.cambiar_rama(RAMA_DEFECTO).unwrap();
        let r = p.merge("feat", "ana", 102).unwrap();
        assert!(matches!(r, ResultadoMerge::FastForward(_)));
        assert_eq!(*p.documento(id).unwrap().atoms[0].content, "feat".to_string());
    }

    #[test]
    fn merge_con_conflicto_de_documento_marca_el_conflicto() {
        let mut p = Proyecto::nuevo("demo");
        let id = p.nuevo_documento("d");
        p.set_documento(id, doc_con("d", &["base"]));
        let h1 = p.push("ana", "base", 100).unwrap();
        // Rama feat: cambia el doc.
        p.rama_nueva("feat", Some(h1));
        p.cambiar_rama("feat").unwrap();
        p.set_documento(id, doc_con("d", &["lado-feat"]));
        p.push("ana", "feat", 101).unwrap();
        // principal: cambia el MISMO doc distinto.
        p.cambiar_rama(RAMA_DEFECTO).unwrap();
        p.set_documento(id, doc_con("d", &["lado-main"]));
        p.push("ana", "main", 102).unwrap();
        // Merge → conflicto en `id`, gana ours (main).
        let r = p.merge("feat", "ana", 103).unwrap();
        match r {
            ResultadoMerge::Merge { conflictos, .. } => {
                assert_eq!(conflictos, vec![id]);
            }
            otro => panic!("esperaba Merge con conflicto, fue {otro:?}"),
        }
        assert_eq!(*p.documento(id).unwrap().atoms[0].content, "lado-main".to_string());
        // El commit de merge tiene 2 padres.
        let hm = p.head_commit().unwrap();
        assert_eq!(p.commit(&hm).unwrap().padres.len(), 2);
    }

    #[test]
    fn diff_detecta_modificado_y_agregado() {
        let mut p = Proyecto::nuevo("demo");
        // Átomos con id estable a través de versiones (como hace el editor real).
        let mut a = NarrativeAtom::new("uno", "es");
        let mut b = NarrativeAtom::new("dos", "es");
        let id = p.nuevo_documento("d");
        let mut d1 = DocEstado::vacio("d");
        let mut c = Cuerpo::nuevo("es", "d", Intencion::Original, 0);
        c.agregar(a.id, 0);
        c.agregar(b.id, 0);
        d1.cuerpos.push(c.clone());
        d1.atoms.push(a.clone());
        d1.atoms.push(b.clone());
        p.set_documento(id, d1);
        let _v1 = p.push("ana", "v1", 100).unwrap();

        // v2: modifica b (mismo id) y agrega c (id nuevo).
        b.set_content("DOS!");
        let cc = NarrativeAtom::new("tres", "es");
        let mut d2 = DocEstado::vacio("d");
        let mut cuerpo2 = Cuerpo::nuevo("es", "d", Intencion::Original, 0);
        cuerpo2.agregar(a.id, 0);
        cuerpo2.agregar(b.id, 0);
        cuerpo2.agregar(cc.id, 0);
        d2.cuerpos.push(cuerpo2);
        d2.atoms.push(a.clone());
        d2.atoms.push(b.clone());
        d2.atoms.push(cc.clone());
        p.set_documento(id, d2);
        let v2 = p.push("ana", "v2", 200).unwrap();

        let padre = p.commit(&v2).unwrap().padres[0];
        let diff = p.diff(Some(padre), v2).unwrap();
        assert_eq!(diff.docs.len(), 1);
        let dd = &diff.docs[0];
        assert!(dd.doc_clase.is_none(), "doc modificado, no agregado");
        let modif: Vec<_> = dd.atomos.iter().filter(|x| x.clase == ClaseDiff::Modificado).collect();
        let agreg: Vec<_> = dd.atomos.iter().filter(|x| x.clase == ClaseDiff::Agregado).collect();
        assert_eq!(modif.len(), 1);
        assert_eq!(modif[0].id, b.id);
        assert_eq!(agreg.len(), 1);
        assert_eq!(agreg[0].id, cc.id);
    }

    #[test]
    fn borrar_rama_y_renombrar_documento() {
        let mut p = Proyecto::nuevo("demo");
        let id = p.nuevo_documento("viejo");
        p.set_documento(id, doc_con("viejo", &["x"]));
        let h1 = p.push("ana", "c1", 100).unwrap();
        p.rama_nueva("feat", Some(h1));
        // No se puede borrar la rama actual (principal).
        assert!(!p.borrar_rama(RAMA_DEFECTO));
        // Sí se puede borrar otra.
        assert!(p.borrar_rama("feat"));
        assert!(p.ramas().iter().all(|(n, _)| n != "feat"));
        // Renombrar documento.
        p.renombrar_documento(id, "nuevo nombre");
        assert_eq!(p.documento(id).unwrap().nombre, "nuevo nombre");
    }

    #[test]
    fn archivo_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("demo.pluma");
        let h;
        let id;
        {
            let mut p = Proyecto::nuevo("demo");
            id = p.nuevo_documento("d");
            p.set_documento(id, doc_con("d", &["contenido"]));
            h = p.push("ana", "c1", 100).unwrap();
            p.guardar(&path).unwrap();
        }
        let p2 = Proyecto::abrir(&path).unwrap();
        assert_eq!(p2.nombre, "demo");
        assert_eq!(p2.head_commit(), Some(h));
        assert_eq!(p2.historia().len(), 1);
        assert_eq!(*p2.documento(id).unwrap().atoms[0].content, "contenido".to_string());
    }

    #[test]
    fn puente_colecciones_ida_y_vuelta() {
        use std::collections::HashMap;
        let mut c = Cuerpo::nuevo("es", "doc", Intencion::Original, 0);
        let a1 = NarrativeAtom::new("uno", "es");
        let a2 = NarrativeAtom::new("dos", "es");
        c.agregar(a1.id, 0);
        c.agregar(a2.id, 0);
        let mut atoms: HashMap<Uuid, NarrativeAtom> = HashMap::new();
        atoms.insert(a1.id, a1.clone());
        atoms.insert(a2.id, a2.clone());
        // Un átomo huérfano (no referenciado) no debe entrar al DocEstado.
        let huerfano = NarrativeAtom::new("huérfano", "es");
        atoms.insert(huerfano.id, huerfano.clone());
        let mut estilos: HashMap<Uuid, EstiloLienzo> = HashMap::new();
        estilos.insert(c.id, EstiloLienzo::nuevo());

        let d = DocEstado::desde_colecciones("doc", &[c.clone()], &atoms, &[], &[], &estilos);
        assert_eq!(d.cuerpos.len(), 1);
        assert_eq!(d.atoms.len(), 2, "sólo los átomos del haz, sin el huérfano");
        assert_eq!(d.estilos.len(), 1);

        let m = d.atoms_map();
        assert!(m.contains_key(&a1.id) && m.contains_key(&a2.id));
        assert!(!m.contains_key(&huerfano.id));
        assert!(d.estilos_map().contains_key(&c.id));
    }

    #[test]
    fn abrir_magia_invalida_falla() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("basura.pluma");
        std::fs::write(&path, b"no soy un pluma").unwrap();
        assert!(matches!(
            Proyecto::abrir(&path),
            Err(ProyectoError::MagiaInvalida) | Err(ProyectoError::Serde(_))
        ));
    }
}
