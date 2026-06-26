//! `multilienzo` — persistencia de cuerpos, transformaciones y cartas.
//!
//! Extiende el store con tres tipos nuevos sobre el mismo `sled::Db`:
//!
//! - `Cuerpo` (`pluma-cuerpo`): el lienzo del haz. Clave = `cuerpo.id`.
//! - `Transformacion` (`pluma-transform`): la receta que deriva un
//!   cuerpo hija de una madre. Clave = `transformacion.id`.
//! - `CartaHebras` (`pluma-align`): las hebras entre dos cuerpos.
//!   Clave = `cuerpo_a.id || cuerpo_b.id` (32 bytes). Si la carta no
//!   tiene `cuerpo_a`/`cuerpo_b` anotados, no se puede persistir.
//!
//! Cada tipo vive en su propio `sled::Tree` para que los iter no se
//! mezclen y para que `len()` por colección tenga sentido. El sled
//! subyacente sigue siendo el mismo `Db` que abre [`crate::GraphStore`]
//! — un solo path en disco contiene todo el estado de pluma.

use sled::{Db, Tree};
use uuid::Uuid;

use serde::{Deserialize, Serialize};

use pluma_align::CartaHebras;
use pluma_cuerpo::Cuerpo;
use pluma_estilo::EstiloLienzo;
use pluma_transform::Transformacion;

use crate::StoreError;

const TREE_ATOMS: &str = "atoms";
const TREE_CUERPOS: &str = "cuerpos";
const TREE_TRANSFORMACIONES: &str = "transformaciones";
const TREE_CARTAS: &str = "cartas";
const TREE_ESTILOS: &str = "estilos";
const TREE_UI: &str = "ui";
const KEY_ESTADO_UI: &[u8] = b"default";

/// Estado de UI persistible — sobrevive a cerrar la app. Por documento
/// hay UN estado (clave fija `b"default"` en el tree "ui"); si el
/// futuro pide multi-documento, las claves se versionan por documento.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EstadoUi {
    /// Focus mode: ocultar todos los cuerpos derivados, solo madre.
    pub solo_madre: bool,
    /// Búsqueda transversal activa (vacío = sin búsqueda).
    pub busqueda: String,
    /// Desplazamiento horizontal del multilienzo, en px.
    pub scroll_x: f32,
    /// Nombre del backend LLM activo — se restaura al reabrir el
    /// documento. Formato libre (`"anthropic"`, `"gemini"`, …). Vacío
    /// = el demo elige por defecto.
    pub backend_llm: String,
}

/// Errores específicos del multilienzo store.
#[derive(Debug, thiserror::Error)]
pub enum MultilienzoError {
    #[error("store: {0}")]
    Store(#[from] StoreError),
    #[error(
        "carta sin par cuerpo_a/cuerpo_b anotado — no se puede persistir; \
         use `CartaHebras::con_par(a, b)` al construirla"
    )]
    CartaSinPar,
}

/// Store unificado para todo el estado del multilienzo. Abre un único
/// `sled::Db` y expone trees nominales para cada tipo.
pub struct PlumaStore {
    db: Db,
    atoms: Tree,
    cuerpos: Tree,
    transformaciones: Tree,
    cartas: Tree,
    estilos: Tree,
    ui: Tree,
}

impl PlumaStore {
    /// Abre (o crea) el store en `path`. Crea los trees nominales.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, MultilienzoError> {
        let db = sled::open(path).map_err(StoreError::from)?;
        let atoms = db.open_tree(TREE_ATOMS).map_err(StoreError::from)?;
        let cuerpos = db.open_tree(TREE_CUERPOS).map_err(StoreError::from)?;
        let transformaciones = db
            .open_tree(TREE_TRANSFORMACIONES)
            .map_err(StoreError::from)?;
        let cartas = db.open_tree(TREE_CARTAS).map_err(StoreError::from)?;
        let estilos = db.open_tree(TREE_ESTILOS).map_err(StoreError::from)?;
        let ui = db.open_tree(TREE_UI).map_err(StoreError::from)?;
        Ok(Self {
            db,
            atoms,
            cuerpos,
            transformaciones,
            cartas,
            estilos,
            ui,
        })
    }

    /// Vacía y persiste todos los cambios pendientes en disco. Útil para
    /// confirmar batches antes de cerrar.
    pub fn flush(&self) -> Result<(), MultilienzoError> {
        self.db.flush().map_err(StoreError::from)?;
        Ok(())
    }

    // ----- Atoms ----------------------------------------------------------

    /// Guarda un `NarrativeAtom`. Sobrescribe si el id ya existía.
    pub fn put_atom(&self, atom: &pluma_core::NarrativeAtom) -> Result<(), MultilienzoError> {
        let bytes = bincode::serialize(atom).map_err(StoreError::from)?;
        self.atoms
            .insert(atom.id.as_bytes(), bytes)
            .map_err(StoreError::from)?;
        Ok(())
    }

    pub fn get_atom(&self, id: Uuid) -> Result<Option<pluma_core::NarrativeAtom>, MultilienzoError> {
        match self.atoms.get(id.as_bytes()).map_err(StoreError::from)? {
            Some(b) => Ok(Some(
                bincode::deserialize(&b).map_err(StoreError::from)?,
            )),
            None => Ok(None),
        }
    }

    /// Itera todos los átomos. El callback recibe `Result` porque la
    /// deserialización puede fallar (datos corruptos) — el iter no se
    /// detiene en el primer error; el caller decide.
    pub fn iter_atoms(
        &self,
    ) -> impl Iterator<Item = Result<pluma_core::NarrativeAtom, MultilienzoError>> + '_ {
        self.atoms.iter().map(|entry| {
            let (_, bytes) = entry.map_err(|e| StoreError::from(e))?;
            bincode::deserialize::<pluma_core::NarrativeAtom>(&bytes)
                .map_err(|e| MultilienzoError::Store(StoreError::from(e)))
        })
    }

    pub fn atoms_len(&self) -> usize {
        self.atoms.len()
    }

    // ----- Cuerpos --------------------------------------------------------

    pub fn put_cuerpo(&self, cuerpo: &Cuerpo) -> Result<(), MultilienzoError> {
        let bytes = bincode::serialize(cuerpo).map_err(StoreError::from)?;
        self.cuerpos
            .insert(cuerpo.id.as_bytes(), bytes)
            .map_err(StoreError::from)?;
        Ok(())
    }

    pub fn get_cuerpo(&self, id: Uuid) -> Result<Option<Cuerpo>, MultilienzoError> {
        match self.cuerpos.get(id.as_bytes()).map_err(StoreError::from)? {
            Some(b) => Ok(Some(
                bincode::deserialize(&b).map_err(StoreError::from)?,
            )),
            None => Ok(None),
        }
    }

    pub fn iter_cuerpos(&self) -> impl Iterator<Item = Result<Cuerpo, MultilienzoError>> + '_ {
        self.cuerpos.iter().map(|entry| {
            let (_, bytes) = entry.map_err(|e| StoreError::from(e))?;
            bincode::deserialize::<Cuerpo>(&bytes)
                .map_err(|e| MultilienzoError::Store(StoreError::from(e)))
        })
    }

    pub fn cuerpos_len(&self) -> usize {
        self.cuerpos.len()
    }

    pub fn remove_cuerpo(&self, id: Uuid) -> Result<(), MultilienzoError> {
        self.cuerpos
            .remove(id.as_bytes())
            .map_err(StoreError::from)?;
        Ok(())
    }

    // ----- Transformaciones ----------------------------------------------

    pub fn put_transformacion(&self, t: &Transformacion) -> Result<(), MultilienzoError> {
        let bytes = bincode::serialize(t).map_err(StoreError::from)?;
        self.transformaciones
            .insert(t.id.as_bytes(), bytes)
            .map_err(StoreError::from)?;
        Ok(())
    }

    pub fn get_transformacion(
        &self,
        id: Uuid,
    ) -> Result<Option<Transformacion>, MultilienzoError> {
        match self
            .transformaciones
            .get(id.as_bytes())
            .map_err(StoreError::from)?
        {
            Some(b) => Ok(Some(
                bincode::deserialize(&b).map_err(StoreError::from)?,
            )),
            None => Ok(None),
        }
    }

    pub fn iter_transformaciones(
        &self,
    ) -> impl Iterator<Item = Result<Transformacion, MultilienzoError>> + '_ {
        self.transformaciones.iter().map(|entry| {
            let (_, bytes) = entry.map_err(|e| StoreError::from(e))?;
            bincode::deserialize::<Transformacion>(&bytes)
                .map_err(|e| MultilienzoError::Store(StoreError::from(e)))
        })
    }

    /// Lista las transformaciones cuyo `madre` es `madre_id`. Scan
    /// lineal — sin índice secundario por ahora (el catálogo es chico
    /// dentro de un documento).
    pub fn transformaciones_de(
        &self,
        madre_id: Uuid,
    ) -> Result<Vec<Transformacion>, MultilienzoError> {
        let mut out = Vec::new();
        for t in self.iter_transformaciones() {
            let t = t?;
            if t.madre == madre_id {
                out.push(t);
            }
        }
        Ok(out)
    }

    // ----- Cartas ---------------------------------------------------------

    /// Persiste una carta. Exige que `carta.cuerpo_a` y `carta.cuerpo_b`
    /// estén anotados — la clave de la carta es el par. Una carta huérfana
    /// no puede persistirse: el dato del par ES la clave.
    pub fn put_carta(&self, carta: &CartaHebras) -> Result<(), MultilienzoError> {
        let key = clave_carta(carta)?;
        let bytes = bincode::serialize(carta).map_err(StoreError::from)?;
        self.cartas.insert(key, bytes).map_err(StoreError::from)?;
        Ok(())
    }

    /// Lee la carta de un par. La búsqueda es por `(cuerpo_a, cuerpo_b)`
    /// EN EL ORDEN dado — si la carta se guardó con orden invertido, no
    /// la encuentra. Para encontrar bidireccionalmente, usar
    /// [`Self::get_carta_bidir`].
    pub fn get_carta(
        &self,
        cuerpo_a: Uuid,
        cuerpo_b: Uuid,
    ) -> Result<Option<CartaHebras>, MultilienzoError> {
        let key = clave_de_par(cuerpo_a, cuerpo_b);
        match self.cartas.get(&key).map_err(StoreError::from)? {
            Some(b) => Ok(Some(
                bincode::deserialize(&b).map_err(StoreError::from)?,
            )),
            None => Ok(None),
        }
    }

    /// Busca la carta en cualquier orden de los cuerpos — primero
    /// `(a,b)`, luego `(b,a)`. Útil cuando el caller no sabe en qué
    /// orden fue construida originalmente.
    pub fn get_carta_bidir(
        &self,
        c1: Uuid,
        c2: Uuid,
    ) -> Result<Option<CartaHebras>, MultilienzoError> {
        if let Some(c) = self.get_carta(c1, c2)? {
            return Ok(Some(c));
        }
        self.get_carta(c2, c1)
    }

    pub fn iter_cartas(
        &self,
    ) -> impl Iterator<Item = Result<CartaHebras, MultilienzoError>> + '_ {
        self.cartas.iter().map(|entry| {
            let (_, bytes) = entry.map_err(|e| StoreError::from(e))?;
            bincode::deserialize::<CartaHebras>(&bytes)
                .map_err(|e| MultilienzoError::Store(StoreError::from(e)))
        })
    }

    pub fn cartas_len(&self) -> usize {
        self.cartas.len()
    }

    // ----- Estilos --------------------------------------------------------

    /// Guarda el estilo de un lienzo. Clave = id del cuerpo. Sobrescribe.
    pub fn put_estilo(&self, cuerpo: Uuid, estilo: &EstiloLienzo) -> Result<(), MultilienzoError> {
        let bytes = bincode::serialize(estilo).map_err(StoreError::from)?;
        self.estilos
            .insert(cuerpo.as_bytes(), bytes)
            .map_err(StoreError::from)?;
        Ok(())
    }

    /// Lee el estilo de un lienzo. `None` si nunca se guardó — el caller cae
    /// a `EstiloLienzo::default()` (lienzo sin estilo, render por defecto).
    pub fn get_estilo(&self, cuerpo: Uuid) -> Result<Option<EstiloLienzo>, MultilienzoError> {
        match self.estilos.get(cuerpo.as_bytes()).map_err(StoreError::from)? {
            Some(b) => Ok(Some(bincode::deserialize(&b).map_err(StoreError::from)?)),
            None => Ok(None),
        }
    }

    /// Itera todos los estilos junto al id del cuerpo al que pertenecen.
    pub fn iter_estilos(
        &self,
    ) -> impl Iterator<Item = Result<(Uuid, EstiloLienzo), MultilienzoError>> + '_ {
        self.estilos.iter().map(|entry| {
            let (k, bytes) = entry.map_err(StoreError::from)?;
            let id = Uuid::from_slice(&k)
                .map_err(|_| MultilienzoError::Store(StoreError::ClaveInvalida))?;
            let estilo = bincode::deserialize::<EstiloLienzo>(&bytes)
                .map_err(|e| MultilienzoError::Store(StoreError::from(e)))?;
            Ok((id, estilo))
        })
    }

    /// Borra el estilo de un lienzo (al eliminar el cuerpo).
    pub fn remove_estilo(&self, cuerpo: Uuid) -> Result<(), MultilienzoError> {
        self.estilos
            .remove(cuerpo.as_bytes())
            .map_err(StoreError::from)?;
        Ok(())
    }

    // ----- Estado UI ------------------------------------------------------

    /// Guarda el estado de UI del documento. Sobrescribe el anterior; el
    /// modelo aquí asume UN estado por store (clave fija).
    pub fn put_estado_ui(&self, e: &EstadoUi) -> Result<(), MultilienzoError> {
        let bytes = bincode::serialize(e).map_err(StoreError::from)?;
        self.ui.insert(KEY_ESTADO_UI, bytes).map_err(StoreError::from)?;
        Ok(())
    }

    /// Lee el estado de UI persistido. `None` si nunca se guardó —
    /// caller cae a `EstadoUi::default()`.
    pub fn get_estado_ui(&self) -> Result<Option<EstadoUi>, MultilienzoError> {
        match self.ui.get(KEY_ESTADO_UI).map_err(StoreError::from)? {
            Some(b) => Ok(Some(
                bincode::deserialize(&b).map_err(StoreError::from)?,
            )),
            None => Ok(None),
        }
    }
}

/// Compone la clave binaria de una carta: 16 bytes de `cuerpo_a` +
/// 16 bytes de `cuerpo_b`. Devuelve error si la carta no tiene par.
fn clave_carta(carta: &CartaHebras) -> Result<[u8; 32], MultilienzoError> {
    match (carta.cuerpo_a, carta.cuerpo_b) {
        (Some(a), Some(b)) => Ok(clave_de_par(a, b)),
        _ => Err(MultilienzoError::CartaSinPar),
    }
}

/// Compone la clave binaria de un par `(cuerpo_a, cuerpo_b)`. El orden
/// importa — distintos órdenes = distintas claves.
fn clave_de_par(a: Uuid, b: Uuid) -> [u8; 32] {
    let mut k = [0u8; 32];
    k[0..16].copy_from_slice(a.as_bytes());
    k[16..32].copy_from_slice(b.as_bytes());
    k
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use pluma_align::{Alineamiento, CartaHebras, OrigenAlineamiento};
    use pluma_core::NarrativeAtom;
    use pluma_cuerpo::{Cuerpo, Intencion};
    use pluma_transform::{TipoTransformacion, Transformacion};

    fn store_temp() -> (PlumaStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let s = PlumaStore::open(dir.path().join("pluma.sled")).unwrap();
        (s, dir)
    }

    #[test]
    fn atom_roundtrip_y_count() {
        let (s, _d) = store_temp();
        let a = NarrativeAtom::new("hola", "es");
        let id = a.id;
        s.put_atom(&a).unwrap();
        let cargado = s.get_atom(id).unwrap().unwrap();
        assert_eq!(*cargado.content, *a.content);
        assert_eq!(s.atoms_len(), 1);
    }

    #[test]
    fn cuerpo_roundtrip_conserva_metadatos_y_orden() {
        let (s, _d) = store_temp();
        let mut c = Cuerpo::nuevo("es", "es (original)", Intencion::Original, 100);
        c.agregar(Uuid::new_v4(), 101);
        c.agregar(Uuid::new_v4(), 102);
        let id = c.id;
        s.put_cuerpo(&c).unwrap();

        let cargado = s.get_cuerpo(id).unwrap().unwrap();
        assert_eq!(cargado, c);
        assert_eq!(s.cuerpos_len(), 1);
    }

    #[test]
    fn cuerpo_remove_devuelve_none() {
        let (s, _d) = store_temp();
        let c = Cuerpo::nuevo("es", "es", Intencion::Original, 0);
        let id = c.id;
        s.put_cuerpo(&c).unwrap();
        s.remove_cuerpo(id).unwrap();
        assert!(s.get_cuerpo(id).unwrap().is_none());
    }

    #[test]
    fn transformacion_roundtrip_y_filtro_por_madre() {
        let (s, _d) = store_temp();
        let madre = Uuid::new_v4();
        let otra_madre = Uuid::new_v4();
        let t1 = Transformacion::nueva(
            madre,
            Uuid::new_v4(),
            TipoTransformacion::Traducir { lengua_destino: "qu".into() },
            "ana",
            1,
        );
        let t2 = Transformacion::nueva(
            madre,
            Uuid::new_v4(),
            TipoTransformacion::Tono { etiqueta: "formal".into() },
            "ana",
            2,
        );
        let t3 = Transformacion::nueva(
            otra_madre,
            Uuid::new_v4(),
            TipoTransformacion::Identidad,
            "ana",
            3,
        );
        s.put_transformacion(&t1).unwrap();
        s.put_transformacion(&t2).unwrap();
        s.put_transformacion(&t3).unwrap();

        let de_madre = s.transformaciones_de(madre).unwrap();
        assert_eq!(de_madre.len(), 2);
        let ids: Vec<Uuid> = de_madre.iter().map(|t| t.id).collect();
        assert!(ids.contains(&t1.id));
        assert!(ids.contains(&t2.id));
        assert!(!ids.contains(&t3.id));
    }

    #[test]
    fn carta_sin_par_no_persiste() {
        let (s, _d) = store_temp();
        let carta = CartaHebras::nueva(); // sin con_par(...)
        match s.put_carta(&carta) {
            Err(MultilienzoError::CartaSinPar) => {}
            otro => panic!("esperaba CartaSinPar, fue {otro:?}"),
        }
    }

    #[test]
    fn carta_con_par_roundtrip_por_clave_y_bidir() {
        let (s, _d) = store_temp();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let mut carta = CartaHebras::nueva().con_par(a, b);
        carta.agregar(Alineamiento::nuevo(
            Uuid::new_v4(),
            Uuid::new_v4(),
            0.8,
            OrigenAlineamiento::Manual { autor: "x".into(), timestamp: 1 },
        ));
        s.put_carta(&carta).unwrap();

        // Por orden directo: la encuentra.
        let r = s.get_carta(a, b).unwrap().unwrap();
        assert_eq!(r.hebras.len(), 1);

        // Por orden invertido (sin bidir): no la encuentra.
        assert!(s.get_carta(b, a).unwrap().is_none());

        // Con bidir: la encuentra.
        assert!(s.get_carta_bidir(b, a).unwrap().is_some());
    }

    #[test]
    fn iter_recupera_todos() {
        let (s, _d) = store_temp();
        for i in 0..5 {
            let c = Cuerpo::nuevo("x", format!("c{i}"), Intencion::Original, i as u64);
            s.put_cuerpo(&c).unwrap();
        }
        let todos: Vec<Cuerpo> = s.iter_cuerpos().collect::<Result<_, _>>().unwrap();
        assert_eq!(todos.len(), 5);
    }

    #[test]
    fn estilo_roundtrip_y_none_si_vacio() {
        use pluma_estilo::{EstiloLienzo, EstiloTexto};
        let (s, _d) = store_temp();
        let cuerpo = Uuid::new_v4();
        // Sin nada guardado: None.
        assert!(s.get_estilo(cuerpo).unwrap().is_none());

        let mut e = EstiloLienzo::nuevo();
        e.set_base(&EstiloTexto {
            color_fg: Some([10, 20, 30, 255]),
            size_px: Some(15.0),
            ..Default::default()
        });
        e.set_zona(1, &EstiloTexto { weight: Some(700.0), ..Default::default() });
        let atom = Uuid::new_v4();
        e.set_span(atom, 0, 4, EstiloTexto { italic: Some(true), ..Default::default() });

        s.put_estilo(cuerpo, &e).unwrap();
        let r = s.get_estilo(cuerpo).unwrap().unwrap();
        assert_eq!(r, e);

        // iter_estilos recupera el par (id, estilo).
        let todos: Vec<_> = s.iter_estilos().collect::<Result<_, _>>().unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].0, cuerpo);

        // remove deja el get en None.
        s.remove_estilo(cuerpo).unwrap();
        assert!(s.get_estilo(cuerpo).unwrap().is_none());
    }

    #[test]
    fn estado_ui_roundtrip_y_default_si_vacio() {
        let (s, _d) = store_temp();
        // Sin nada guardado, get devuelve None.
        assert!(s.get_estado_ui().unwrap().is_none());
        // Put + get devuelve el estado intacto.
        let e = EstadoUi {
            solo_madre: true,
            busqueda: "cóndor".to_string(),
            scroll_x: 240.5,
            backend_llm: "gemini".to_string(),
        };
        s.put_estado_ui(&e).unwrap();
        let r = s.get_estado_ui().unwrap().unwrap();
        assert_eq!(r, e);
    }

    #[test]
    fn flush_no_pierde_dato_tras_reabrir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pluma.sled");
        let id = {
            let s = PlumaStore::open(&path).unwrap();
            let c = Cuerpo::nuevo("es", "es", Intencion::Original, 0);
            let id = c.id;
            s.put_cuerpo(&c).unwrap();
            s.flush().unwrap();
            id
        };
        // Reabrir el mismo path.
        let s2 = PlumaStore::open(&path).unwrap();
        assert!(s2.get_cuerpo(id).unwrap().is_some());
        assert_eq!(s2.cuerpos_len(), 1);
    }
}
