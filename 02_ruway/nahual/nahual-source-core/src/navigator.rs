//! [`Navigator`] — el estado de navegación genérico sobre cualquier
//! [`Source`].
//!
//! Es el equivalente agnóstico de `FileExplorerState` de
//! `nahual-file-explorer-llimphi`: mantiene una pila de contenedores (la
//! ruta desde la raíz), los hijos del contenedor actual, la selección y la
//! ventana de scroll virtual. No sabe nada de Llimphi ni de `PathBuf` — sólo
//! de [`Node`]s. El shell lo monta sobre un `Box<dyn Source>` y lo pinta.
//!
//! El punto de Brahman Fase 3: el shell deja de hablar POSIX y pasa a navegar
//! `dyn Source`. Montar una imagen wawa = `Navigator::open(Box::new(
//! WawaImgSource::abrir(...)?))` — el mismo navegador, otro backend.

use std::cmp::Ordering;
use std::io;

use crate::{Node, NodeId, NodeKind, Source};

/// Cuántas filas se ven a la vez por defecto (mismo calibrado que el
/// explorador POSIX histórico).
pub const DEFAULT_VISIBLE_ROWS: usize = 32;

/// Resultado de [`Navigator::open_selected`].
pub enum Opened {
    /// Era un contenedor: ya se descendió a él.
    Descended,
    /// Era una hoja: su [`NodeId`] para que el caller lea sus bytes.
    Leaf(NodeId),
}

/// Por qué columna se ordenan los hijos del contenedor actual.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    /// Alfabético por nombre (default, case-insensitive).
    #[default]
    Name,
    /// Por tamaño en bytes.
    Size,
    /// Por última modificación.
    Mtime,
    /// Por naturaleza del nodo (dir/file/symlink…).
    Kind,
}

/// Dirección del orden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    fn toggle(self) -> Self {
        match self {
            SortDir::Asc => SortDir::Desc,
            SortDir::Desc => SortDir::Asc,
        }
    }
}

impl SortKey {
    /// Dirección natural al elegir esta columna por primera vez: nombre y tipo
    /// ascendentes; tamaño y fecha descendentes (lo grande/lo nuevo primero,
    /// como en un file manager típico).
    fn default_dir(self) -> SortDir {
        match self {
            SortKey::Name | SortKey::Kind => SortDir::Asc,
            SortKey::Size | SortKey::Mtime => SortDir::Desc,
        }
    }
}

/// Cómo presenta el caller los hijos (lista simple vs grilla detalle). Vive
/// acá para que sobreviva a descender/subir y se comparta entre fuentes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    /// Una columna (nombre). El modo histórico.
    #[default]
    List,
    /// Grilla con columnas nombre/tamaño/fecha/tipo (vista detalle Dopus).
    Details,
}

/// Orden de un `NodeKind` para la columna "tipo": contenedores sintéticos y
/// dirs arriba, luego archivos, symlinks, archives.
fn kind_rank(k: NodeKind) -> u8 {
    match k {
        NodeKind::Dir => 0,
        NodeKind::Synthetic => 1,
        NodeKind::Archive => 2,
        NodeKind::File => 3,
        NodeKind::Symlink => 4,
    }
}

/// Estado de navegación sobre una [`Source`]. El caller lo guarda en su
/// modelo y le pasa los eventos de teclado/click.
pub struct Navigator {
    source: Box<dyn Source>,
    /// Contenedores desde la raíz hasta el actual (el último es el actual).
    stack: Vec<Node>,
    /// Hijos del contenedor actual.
    children: Vec<Node>,
    pub selected: usize,
    pub visible_offset: usize,
    pub visible_rows: usize,
    wheel_accum: f32,
    /// Columna y dirección de orden de los hijos del contenedor actual.
    sort_key: SortKey,
    sort_dir: SortDir,
    /// Modo de presentación (lista vs detalle). El widget lo lee.
    pub view: ViewMode,
    /// Filtro vivo por substring del nombre (case-insensitive). Vacío = todo
    /// visible.
    filter: String,
}

impl Navigator {
    /// Monta el navegador sobre una fuente, posándose en su raíz y cargando
    /// los hijos. Error si la raíz no se puede listar.
    pub fn open(source: Box<dyn Source>) -> io::Result<Self> {
        let root = source.root();
        let children = source.children(&root.id)?;
        let mut nav = Self {
            source,
            stack: vec![root],
            children,
            selected: 0,
            visible_offset: 0,
            visible_rows: DEFAULT_VISIBLE_ROWS,
            wheel_accum: 0.0,
            sort_key: SortKey::default(),
            sort_dir: SortKey::default().default_dir(),
            view: ViewMode::default(),
            filter: String::new(),
        };
        nav.apply_sort();
        Ok(nav)
    }

    /// Monta el navegador con una **pila de contenedores ya provista** (de la
    /// raíz al actual), en vez de posarse sólo en la raíz. Sirve para arrancar
    /// adentro de un subárbol con la miga de pan completa — p. ej. POSIX con la
    /// fuente anclada en `/` pero arrancando en el cwd, sin perder la cadena de
    /// ancestros para el breadcrumb ni la navegación hacia arriba.
    ///
    /// `stack` debe ser no vacía; el último es el contenedor actual, cuyos
    /// hijos se cargan. El caller arma la cadena (para POSIX es trivial: partir
    /// la ruta en componentes).
    pub fn open_at(source: Box<dyn Source>, stack: Vec<Node>) -> io::Result<Self> {
        let current = stack.last().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "open_at: pila vacía")
        })?;
        let children = source.children(&current.id)?;
        let mut nav = Self {
            source,
            stack,
            children,
            selected: 0,
            visible_offset: 0,
            visible_rows: DEFAULT_VISIBLE_ROWS,
            wheel_accum: 0.0,
            sort_key: SortKey::default(),
            sort_dir: SortKey::default().default_dir(),
            view: ViewMode::default(),
            filter: String::new(),
        };
        nav.apply_sort();
        Ok(nav)
    }

    /// El [`NodeId`] del contenedor actual (el tope de la pila). Para POSIX es
    /// la ruta del directorio en que estamos parados.
    pub fn current_id(&self) -> &NodeId {
        &self.stack.last().expect("la pila nunca está vacía").id
    }

    /// La cadena de contenedores de la raíz al actual — para pintar el
    /// breadcrumb (cada nivel es clicable hacia [`Navigator::ascend_to`]).
    pub fn ancestors(&self) -> &[Node] {
        &self.stack
    }

    /// Sube directo al ancestro en la posición `depth` de la pila (0 = raíz).
    /// `false` si `depth` ya es el nivel actual (o está fuera de rango); recarga
    /// los hijos de ese nivel y reubica la selección al inicio.
    pub fn ascend_to(&mut self, depth: usize) -> io::Result<bool> {
        if depth + 1 >= self.stack.len() {
            return Ok(false);
        }
        self.stack.truncate(depth + 1);
        let actual = self.stack.last().expect("queda al menos la raíz");
        self.children = self.source.children(&actual.id)?;
        self.apply_sort();
        self.selected = 0;
        self.visible_offset = 0;
        Ok(true)
    }

    /// Nombre humano de la fuente (para el header).
    pub fn label(&self) -> String {
        self.source.label()
    }

    /// Ruta de nombres desde la raíz al contenedor actual, " / "-separada.
    pub fn breadcrumb(&self) -> String {
        self.stack
            .iter()
            .map(|n| n.name.as_str())
            .collect::<Vec<_>>()
            .join(" / ")
    }

    /// Hijos del contenedor actual.
    pub fn children(&self) -> &[Node] {
        &self.children
    }

    /// `true` si estamos en la raíz (no hay a dónde subir dentro de la
    /// fuente). El caller lo usa para decidir si "subir" desmonta la fuente.
    pub fn at_root(&self) -> bool {
        self.stack.len() <= 1
    }

    /// El nodo actualmente seleccionado.
    pub fn selected_node(&self) -> Option<&Node> {
        self.children.get(self.selected)
    }

    /// Lee los bytes de una hoja por su id (delega en la fuente).
    pub fn read(&self, id: &NodeId) -> io::Result<Vec<u8>> {
        self.source.read(id)
    }

    /// Mueve la selección a la fila visible anterior (salta lo filtrado).
    pub fn up(&mut self) -> bool {
        let Some(prev) = (0..self.selected).rev().find(|&i| self.passes(&self.children[i])) else {
            return false;
        };
        self.selected = prev;
        self.sync_offset();
        true
    }

    /// Mueve la selección a la fila visible siguiente (salta lo filtrado).
    pub fn down(&mut self) -> bool {
        let Some(next) =
            (self.selected + 1..self.children.len()).find(|&i| self.passes(&self.children[i]))
        else {
            return false;
        };
        self.selected = next;
        self.sync_offset();
        true
    }

    /// Selecciona la fila `idx` (con bound check + scroll sync).
    pub fn select(&mut self, idx: usize) -> bool {
        if idx >= self.children.len() {
            return false;
        }
        self.selected = idx;
        self.sync_offset();
        true
    }

    /// Abre la selección: si es contenedor desciende; si es hoja devuelve su
    /// id. `None` si no hay selección. Error si el contenedor no se puede
    /// listar.
    pub fn open_selected(&mut self) -> io::Result<Option<Opened>> {
        let Some(node) = self.children.get(self.selected).cloned() else {
            return Ok(None);
        };
        if node.is_container {
            let children = self.source.children(&node.id)?;
            self.stack.push(node);
            self.children = children;
            self.apply_sort();
            self.selected = 0;
            self.visible_offset = 0;
            Ok(Some(Opened::Descended))
        } else {
            Ok(Some(Opened::Leaf(node.id)))
        }
    }

    /// Sube al contenedor padre dentro de la fuente. `false` si ya estábamos
    /// en la raíz — el caller interpreta eso como "desmontar la fuente".
    /// Al subir, re-selecciona el contenedor del que veníamos.
    pub fn parent(&mut self) -> io::Result<bool> {
        if self.stack.len() <= 1 {
            return Ok(false);
        }
        let dejado = self.stack.pop().expect("len > 1");
        let actual = self.stack.last().expect("queda al menos la raíz");
        self.children = self.source.children(&actual.id)?;
        self.apply_sort();
        self.selected = self
            .children
            .iter()
            .position(|n| n.id == dejado.id)
            .unwrap_or(0);
        self.visible_offset = 0;
        self.sync_offset();
        Ok(true)
    }

    /// Aplica un delta de rueda (en líneas), devuelve los pasos enteros.
    pub fn apply_wheel(&mut self, delta_y: f32) -> i32 {
        let total = self.wheel_accum + delta_y;
        let steps = total.trunc() as i32;
        self.wheel_accum = total - steps as f32;
        if steps != 0 {
            self.scroll(steps);
        }
        steps
    }

    /// Scroll por N pasos (positivo = abajo). No mueve la selección.
    pub fn scroll(&mut self, steps: i32) {
        if steps == 0 {
            return;
        }
        let max_offset = self.children.len().saturating_sub(self.visible_rows);
        if steps > 0 {
            self.visible_offset = (self.visible_offset + steps as usize).min(max_offset);
        } else {
            self.visible_offset = self.visible_offset.saturating_sub((-steps) as usize);
        }
    }

    // =================================================================
    // Orden, modo de vista y filtro (Fase 4.1)
    // =================================================================

    /// Columna y dirección de orden activas (para la flecha del encabezado).
    pub fn sort(&self) -> (SortKey, SortDir) {
        (self.sort_key, self.sort_dir)
    }

    /// Elige la columna de orden: si ya era la activa, invierte la dirección;
    /// si no, la activa con su dirección natural. Re-ordena preservando qué
    /// nodo estaba seleccionado.
    pub fn set_sort(&mut self, key: SortKey) {
        if self.sort_key == key {
            self.sort_dir = self.sort_dir.toggle();
        } else {
            self.sort_key = key;
            self.sort_dir = key.default_dir();
        }
        let sel_id = self.children.get(self.selected).map(|n| n.id.clone());
        self.apply_sort();
        if let Some(id) = sel_id {
            if let Some(pos) = self.children.iter().position(|n| n.id == id) {
                self.selected = pos;
            }
        }
        self.sync_offset();
    }

    /// Ordena `self.children` in situ según `sort_key`/`sort_dir`. Los
    /// contenedores van SIEMPRE arriba (agrupados), sin importar la dirección
    /// — convención de file manager; dentro de cada grupo manda la columna.
    fn apply_sort(&mut self) {
        let key = self.sort_key;
        let dir = self.sort_dir;
        self.children.sort_by(|a, b| {
            // Grupo: contenedores primero (no se invierte con la dirección).
            let grupo = b.is_container.cmp(&a.is_container);
            if grupo != Ordering::Equal {
                return grupo;
            }
            let base = match key {
                SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortKey::Size => a.size.unwrap_or(0).cmp(&b.size.unwrap_or(0)),
                SortKey::Mtime => a.mtime.unwrap_or(0).cmp(&b.mtime.unwrap_or(0)),
                SortKey::Kind => kind_rank(a.kind).cmp(&kind_rank(b.kind)),
            };
            let base = match dir {
                SortDir::Asc => base,
                SortDir::Desc => base.reverse(),
            };
            // Desempate estable por nombre ascendente.
            base.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
    }

    /// El filtro vivo actual (substring del nombre).
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Fija el filtro y reubica la selección al primer nodo visible si el
    /// seleccionado quedó oculto.
    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        if !self.children.is_empty() && !self.passes(&self.children[self.selected.min(self.children.len() - 1)]) {
            if let Some(i) = (0..self.children.len()).find(|&i| self.passes(&self.children[i])) {
                self.selected = i;
            }
        }
        self.visible_offset = 0;
        self.sync_offset();
    }

    /// `true` si el nodo pasa el filtro vivo.
    fn passes(&self, n: &Node) -> bool {
        self.filter.is_empty() || n.name.to_lowercase().contains(&self.filter.to_lowercase())
    }

    /// Los hijos visibles (que pasan el filtro), apareados con su índice real
    /// en `children` — el caller usa ese índice para `select`/`Msg`. El orden
    /// es el del `apply_sort`.
    pub fn visible(&self) -> Vec<(usize, &Node)> {
        self.children
            .iter()
            .enumerate()
            .filter(|(_, n)| self.passes(n))
            .collect()
    }

    /// Cuántos hijos pasan el filtro.
    pub fn visible_count(&self) -> usize {
        if self.filter.is_empty() {
            self.children.len()
        } else {
            self.children.iter().filter(|n| self.passes(n)).count()
        }
    }

    fn sync_offset(&mut self) {
        if self.selected < self.visible_offset {
            self.visible_offset = self.selected;
        }
        let bottom = self.visible_offset + self.visible_rows;
        if self.selected >= bottom {
            self.visible_offset = self.selected + 1 - self.visible_rows;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PosixSource;
    use std::fs;
    use std::io::Write;

    fn arbol() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        let mut f = fs::File::create(dir.path().join("sub/hoja.txt")).unwrap();
        f.write_all(b"bytes de la hoja").unwrap();
        fs::File::create(dir.path().join("raiz.txt")).unwrap();
        dir
    }

    #[test]
    fn desciende_lee_y_sube() {
        let dir = arbol();
        let mut nav = Navigator::open(Box::new(PosixSource::new(dir.path()))).unwrap();
        assert!(nav.at_root());
        // raíz: "sub/" (dir) primero, luego "raiz.txt".
        assert_eq!(nav.children()[0].name, "sub");
        assert!(nav.children()[0].is_container);

        // Descender a "sub".
        nav.select(0);
        match nav.open_selected().unwrap() {
            Some(Opened::Descended) => {}
            _ => panic!("esperaba descender al dir"),
        }
        assert!(!nav.at_root());
        assert_eq!(nav.breadcrumb().split(" / ").count(), 2);

        // En "sub" hay una hoja: abrirla devuelve su id, y read da bytes.
        let hoja = nav.children().iter().position(|n| n.name == "hoja.txt").unwrap();
        nav.select(hoja);
        match nav.open_selected().unwrap() {
            Some(Opened::Leaf(id)) => {
                assert_eq!(nav.read(&id).unwrap(), b"bytes de la hoja");
            }
            _ => panic!("esperaba una hoja"),
        }

        // Subir vuelve a la raíz y re-selecciona "sub".
        assert!(nav.parent().unwrap());
        assert!(nav.at_root());
        assert_eq!(nav.selected_node().unwrap().name, "sub");
        // Subir desde la raíz = false (el caller desmonta).
        assert!(!nav.parent().unwrap());
    }

    #[test]
    fn navegacion_vacia_no_panickea() {
        let dir = tempfile::tempdir().unwrap();
        let mut nav = Navigator::open(Box::new(PosixSource::new(dir.path()))).unwrap();
        assert!(nav.children().is_empty());
        assert!(!nav.up());
        assert!(!nav.down());
        assert!(nav.open_selected().unwrap().is_none());
    }

    /// Arma un dir con tres archivos de tamaños distintos + un subdir.
    fn arbol_tamanos() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("zdir")).unwrap();
        fs::write(dir.path().join("grande.txt"), vec![b'x'; 300]).unwrap();
        fs::write(dir.path().join("medio.txt"), vec![b'x'; 200]).unwrap();
        fs::write(dir.path().join("chico.txt"), vec![b'x'; 100]).unwrap();
        dir
    }

    #[test]
    fn ordena_por_tamano_con_dirs_arriba() {
        let dir = arbol_tamanos();
        let mut nav = Navigator::open(Box::new(PosixSource::new(dir.path()))).unwrap();
        nav.set_sort(SortKey::Size); // default_dir = Desc (grande primero)
        let nombres: Vec<&str> = nav.children().iter().map(|n| n.name.as_str()).collect();
        // El dir siempre arriba; luego archivos por tamaño descendente.
        assert_eq!(nombres, vec!["zdir", "grande.txt", "medio.txt", "chico.txt"]);
        // Invertir: mismo dir arriba, archivos ascendentes.
        nav.set_sort(SortKey::Size);
        let nombres: Vec<&str> = nav.children().iter().map(|n| n.name.as_str()).collect();
        assert_eq!(nombres, vec!["zdir", "chico.txt", "medio.txt", "grande.txt"]);
    }

    #[test]
    fn set_sort_preserva_seleccion_por_id() {
        let dir = arbol_tamanos();
        let mut nav = Navigator::open(Box::new(PosixSource::new(dir.path()))).unwrap();
        // Seleccionar "chico.txt" (orden alfabético: zdir, chico, grande, medio).
        let idx = nav.children().iter().position(|n| n.name == "chico.txt").unwrap();
        nav.select(idx);
        nav.set_sort(SortKey::Size);
        // Tras reordenar, la selección sigue sobre "chico.txt".
        assert_eq!(nav.selected_node().unwrap().name, "chico.txt");
    }

    #[test]
    fn open_at_arranca_adentro_con_miga_completa() {
        // Fuente anclada en la raíz del tmp, pero arrancamos en sub/ con la
        // cadena de ancestros (raíz · sub).
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/x.txt"), b"x").unwrap();
        let root_id = dir.path().to_string_lossy().into_owned();
        let sub_id = dir.path().join("sub").to_string_lossy().into_owned();
        let stack = vec![
            Node::new(root_id.clone(), "raíz", true),
            Node::new(sub_id.clone(), "sub", true),
        ];
        let mut nav = Navigator::open_at(Box::new(PosixSource::new(dir.path())), stack).unwrap();
        // Estamos en sub/: vemos x.txt, y el breadcrumb tiene los dos niveles.
        assert_eq!(nav.current_id(), &sub_id);
        assert_eq!(nav.breadcrumb().split(" / ").count(), 2);
        assert!(nav.children().iter().any(|n| n.name == "x.txt"));
        // Subir vuelve a la raíz; subir de nuevo = false (tope de la pila).
        assert!(nav.parent().unwrap());
        assert_eq!(nav.current_id(), &root_id);
        assert!(!nav.parent().unwrap());
    }

    #[test]
    fn ascend_to_salta_a_un_ancestro() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a/b/c")).unwrap();
        let ids: Vec<String> = ["", "a", "a/b", "a/b/c"]
            .iter()
            .map(|s| dir.path().join(s).to_string_lossy().into_owned())
            .collect();
        let stack: Vec<Node> = ids
            .iter()
            .zip(["raíz", "a", "b", "c"])
            .map(|(id, name)| Node::new(id.clone(), name, true))
            .collect();
        let mut nav = Navigator::open_at(Box::new(PosixSource::new(dir.path())), stack).unwrap();
        // Estamos en a/b/c (4 niveles). Subir al nivel 1 (a/).
        assert_eq!(nav.ancestors().len(), 4);
        assert!(nav.ascend_to(1).unwrap());
        assert_eq!(nav.current_id(), &ids[1]);
        assert_eq!(nav.ancestors().len(), 2);
        // Saltar al nivel actual = false (no se mueve).
        assert!(!nav.ascend_to(1).unwrap());
    }

    #[test]
    fn filtro_oculta_y_navega_solo_visibles() {
        let dir = arbol_tamanos();
        let mut nav = Navigator::open(Box::new(PosixSource::new(dir.path()))).unwrap();
        nav.set_filter("medio".into());
        assert_eq!(nav.visible_count(), 1);
        let vis = nav.visible();
        assert_eq!(vis.len(), 1);
        assert_eq!(vis[0].1.name, "medio.txt");
        // down no se mueve si sólo hay un visible.
        nav.select(vis[0].0);
        assert!(!nav.down());
        // Limpiar el filtro restaura todo.
        nav.set_filter(String::new());
        assert_eq!(nav.visible_count(), 4);
    }
}
