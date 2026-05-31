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

use std::io;

use crate::{Node, NodeId, Source};

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
}

impl Navigator {
    /// Monta el navegador sobre una fuente, posándose en su raíz y cargando
    /// los hijos. Error si la raíz no se puede listar.
    pub fn open(source: Box<dyn Source>) -> io::Result<Self> {
        let root = source.root();
        let children = source.children(&root.id)?;
        Ok(Self {
            source,
            stack: vec![root],
            children,
            selected: 0,
            visible_offset: 0,
            visible_rows: DEFAULT_VISIBLE_ROWS,
            wheel_accum: 0.0,
        })
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

    /// Mueve la selección una fila arriba.
    pub fn up(&mut self) -> bool {
        if self.selected == 0 {
            return false;
        }
        self.selected -= 1;
        self.sync_offset();
        true
    }

    /// Mueve la selección una fila abajo.
    pub fn down(&mut self) -> bool {
        if self.selected + 1 >= self.children.len() {
            return false;
        }
        self.selected += 1;
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
}
