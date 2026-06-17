//! Menú raíz estilo openbox: un pop-up que aparece al click derecho sobre el
//! fondo del escritorio (no sobre una ventana) y lista comandos del usuario,
//! con **submenús anidados** a cualquier profundidad.
//!
//! Sólo geometría, árbol e hit-testing puros — el render y el input viven en
//! [`crate::drm_backend`]. El árbol de entradas ([`MenuNode`]) se arma desde la
//! config ([`mirada_brain::Config::menu`]) en `main.rs`.
//!
//! El estado abierto es una **cascada de columnas**: la columna 0 muestra la
//! raíz; [`RootMenu::path`] lista, por nivel, qué fila-submenú está abierta, y
//! cada entrada de `path` agrega una columna a la derecha (o a la izquierda si
//! no hay lugar). Mover el puntero sobre una fila-submenú abre su columna hija
//! ([`RootMenu::update_hover`]); el click lanza una hoja o no hace nada sobre un
//! submenú ([`RootMenu::click`]).

/// Alto de cada fila del menú, en píxeles.
pub const ITEM_H: i32 = 26;
/// Ancho de cada columna del menú, en píxeles.
pub const MENU_W: i32 = 210;
/// Relleno vertical arriba y abajo de cada lista, en píxeles.
pub const PAD: i32 = 5;

/// Un nodo del árbol del menú: una **hoja** que lanza `command`, o un
/// **submenú** (cuando `command` es `None`) con `children`.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuNode {
    pub label: String,
    /// `Some(cmd)` = hoja que lanza `cmd`; `None` = submenú.
    pub command: Option<String>,
    /// Hijas del submenú (vacío en una hoja).
    pub children: Vec<MenuNode>,
}

impl MenuNode {
    /// Una hoja con etiqueta y comando.
    pub fn leaf(label: impl Into<String>, command: impl Into<String>) -> Self {
        Self { label: label.into(), command: Some(command.into()), children: vec![] }
    }

    /// Un submenú con etiqueta e hijas.
    pub fn submenu(label: impl Into<String>, children: Vec<MenuNode>) -> Self {
        Self { label: label.into(), command: None, children }
    }

    /// `true` si es un submenú (sin comando, abre una columna hija).
    pub fn is_submenu(&self) -> bool {
        self.command.is_none()
    }
}

/// Entradas del **menú contextual de ventana** (click derecho en el titlebar).
/// Sus comandos llevan el prefijo `@win:` que el backend intercepta (no son
/// comandos de shell): `min`/`max`/`float`/`close` y `ws:<n>` para enviarla a
/// un escritorio. `workspaces` = cuántos escritorios listar en «Enviar a…».
pub fn window_menu_entries(workspaces: usize) -> Vec<MenuNode> {
    let destinos = (0..workspaces)
        .map(|i| MenuNode::leaf(format!("Escritorio {}", i + 1), format!("@win:ws:{i}")))
        .collect();
    vec![
        MenuNode::leaf("Minimizar", "@win:min"),
        MenuNode::leaf("Maximizar / restaurar", "@win:max"),
        MenuNode::leaf("Flotar / teselar", "@win:float"),
        MenuNode::submenu("Enviar a…", destinos),
        MenuNode::leaf("Cerrar", "@win:close"),
    ]
}

/// Lo que devuelve [`RootMenu::click`]: qué hacer con el click.
#[derive(Debug, PartialEq)]
pub enum ClickResult {
    /// Click sobre una hoja: lanzar este comando y cerrar el menú.
    Launch(String),
    /// Click sobre una fila-submenú: el menú sigue abierto.
    Stay,
    /// Click fuera de toda columna: cerrar el menú.
    Close,
}

/// Geometría de una columna ya colocada en pantalla.
struct Col {
    x: i32,
    y: i32,
    len: usize,
}

/// Una fila lista para pintar.
pub struct MenuRowView {
    pub x: i32,
    pub y: i32,
    pub label: String,
    /// Resaltada (bajo el puntero o submenú abierto en esta columna).
    pub highlighted: bool,
    /// Es un submenú (se le pinta un indicador `›`).
    pub submenu: bool,
}

/// Una columna lista para pintar.
pub struct MenuColView {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub rows: Vec<MenuRowView>,
}

/// Un menú raíz abierto: el árbol, el origen de la primera columna y la ruta de
/// submenús abiertos.
pub struct RootMenu {
    root: Vec<MenuNode>,
    ox: i32,
    oy: i32,
    /// `path[k]` = índice de la fila-submenú abierta en la columna `k`.
    path: Vec<usize>,
    out_w: i32,
    out_h: i32,
}

impl RootMenu {
    /// Abre el menú con su primera columna anclada en `(px, py)`, dentro de una
    /// salida de `(out_w, out_h)`.
    pub fn open(px: i32, py: i32, root: Vec<MenuNode>, out_w: i32, out_h: i32) -> Self {
        Self { root, ox: px, oy: py, path: Vec::new(), out_w, out_h }
    }

    /// Alto total de una columna con `n` filas.
    pub fn height_for(n: usize) -> i32 {
        n as i32 * ITEM_H + 2 * PAD
    }

    /// Las entradas que muestra la columna `c` (0 = raíz), siguiendo `path`.
    /// `None` si `c` excede la cascada abierta o la ruta es inconsistente.
    fn column_entries(&self, c: usize) -> Option<&[MenuNode]> {
        let mut nodes = self.root.as_slice();
        for k in 0..c {
            let i = *self.path.get(k)?;
            let node = nodes.get(i)?;
            if node.children.is_empty() {
                return None;
            }
            nodes = node.children.as_slice();
        }
        Some(nodes)
    }

    /// Coloca cada columna abierta: la 0 anclada (acotada) al origen; cada hija
    /// a la derecha de su padre, o a la izquierda si no hay lugar, alineada a la
    /// fila que la abrió. Acotadas a la salida.
    fn columns(&self) -> Vec<Col> {
        let mut cols = Vec::new();
        let (mut prev_x, mut prev_y) = (0, 0);
        for c in 0..=self.path.len() {
            let Some(entries) = self.column_entries(c) else { break };
            let len = entries.len();
            let h = Self::height_for(len);
            let (x, y) = if c == 0 {
                let x = self.ox.min((self.out_w - MENU_W).max(0)).max(0);
                let y = self.oy.min((self.out_h - h).max(0)).max(0);
                (x, y)
            } else {
                let prow = self.path[c - 1];
                let right = prev_x + MENU_W;
                let x = if right + MENU_W <= self.out_w {
                    right
                } else if prev_x - MENU_W >= 0 {
                    prev_x - MENU_W
                } else {
                    (self.out_w - MENU_W).max(0)
                };
                let row_y = prev_y + PAD + prow as i32 * ITEM_H;
                let y = row_y.min((self.out_h - h).max(0)).max(0);
                (x, y)
            };
            cols.push(Col { x, y, len });
            prev_x = x;
            prev_y = y;
        }
        cols
    }

    /// La `(columna, fila)` bajo `(px, py)`, o `None`. Itera de la columna más
    /// profunda a la raíz: si una hija solapa a su padre (colocada a la
    /// izquierda), gana la hija (está encima).
    fn hit(&self, px: i32, py: i32) -> Option<(usize, usize)> {
        let cols = self.columns();
        for (c, col) in cols.iter().enumerate().rev() {
            if px >= col.x && px < col.x + MENU_W {
                let rel = py - (col.y + PAD);
                if rel >= 0 && rel < col.len as i32 * ITEM_H {
                    return Some((c, (rel / ITEM_H) as usize));
                }
            }
        }
        None
    }

    /// Actualiza la cascada según el puntero: sobre una fila-submenú abre su
    /// columna hija (cerrando las más profundas); sobre una hoja cierra las
    /// columnas posteriores a la suya; fuera de todo, no toca nada (no colapsa
    /// al cruzar el hueco entre columnas).
    pub fn update_hover(&mut self, px: i32, py: i32) {
        let action = self.hit(px, py).and_then(|(c, r)| {
            self.column_entries(c)
                .and_then(|e| e.get(r))
                .map(|n| (c, r, n.is_submenu()))
        });
        if let Some((c, r, is_sub)) = action {
            self.path.truncate(c);
            if is_sub {
                self.path.push(r);
            }
        }
    }

    /// Resuelve un click: hoja → lanzar; fila-submenú → abrirla y seguir; fuera
    /// de toda columna → cerrar.
    pub fn click(&mut self, px: i32, py: i32) -> ClickResult {
        let Some((c, r)) = self.hit(px, py) else {
            return ClickResult::Close;
        };
        let cmd = self
            .column_entries(c)
            .and_then(|e| e.get(r))
            .and_then(|n| n.command.clone());
        match cmd {
            Some(cmd) => ClickResult::Launch(cmd),
            None => {
                // Fila-submenú: asegurar que su columna hija esté abierta.
                self.path.truncate(c);
                self.path.push(r);
                ClickResult::Stay
            }
        }
    }

    /// Las columnas listas para pintar, con la fila bajo `(px, py)` y las
    /// fila-submenú abiertas resaltadas.
    pub fn render(&self, px: i32, py: i32) -> Vec<MenuColView> {
        let hover = self.hit(px, py);
        let cols = self.columns();
        let mut out = Vec::new();
        for (c, col) in cols.iter().enumerate() {
            let Some(entries) = self.column_entries(c) else { continue };
            let rows = entries
                .iter()
                .enumerate()
                .map(|(r, node)| {
                    let ry = col.y + PAD + r as i32 * ITEM_H;
                    let on_path = self.path.get(c).copied() == Some(r);
                    let hovered = hover == Some((c, r));
                    MenuRowView {
                        x: col.x,
                        y: ry,
                        label: node.label.clone(),
                        highlighted: on_path || hovered,
                        submenu: node.is_submenu(),
                    }
                })
                .collect();
            out.push(MenuColView {
                x: col.x,
                y: col.y,
                w: MENU_W,
                h: Self::height_for(col.len),
                rows,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> Vec<MenuNode> {
        vec![
            MenuNode::leaf("Terminal", "kitty"),
            MenuNode::submenu(
                "Apps",
                vec![
                    MenuNode::leaf("Navegador", "firefox"),
                    MenuNode::submenu("Más", vec![MenuNode::leaf("nada", "nada")]),
                ],
            ),
        ]
    }

    fn menu() -> RootMenu {
        RootMenu::open(100, 100, tree(), 1920, 1080)
    }

    fn row_y(col_y: i32, r: i32) -> i32 {
        col_y + PAD + r * ITEM_H + 1
    }

    #[test]
    fn arranca_con_una_sola_columna() {
        let m = menu();
        assert_eq!(m.columns().len(), 1);
    }

    #[test]
    fn hover_sobre_submenu_abre_columna_hija() {
        let mut m = menu();
        // Fila 1 (Apps) de la columna 0 — es submenú.
        m.update_hover(150, row_y(100, 1));
        assert_eq!(m.path, vec![1]);
        assert_eq!(m.columns().len(), 2);
    }

    #[test]
    fn hover_sobre_hoja_cierra_las_columnas_profundas() {
        let mut m = menu();
        m.update_hover(150, row_y(100, 1)); // abre Apps
        assert_eq!(m.path, vec![1]);
        // Ahora hover sobre la hoja "Terminal" (fila 0 de la columna 0).
        m.update_hover(150, row_y(100, 0));
        assert!(m.path.is_empty(), "la hoja debe colapsar la cascada");
    }

    #[test]
    fn cascada_de_dos_niveles() {
        let mut m = menu();
        m.update_hover(150, row_y(100, 1)); // Apps → col 1
        let cols = m.columns();
        assert_eq!(cols.len(), 2);
        // En la columna 1, la fila 1 ("Más") es submenú: hover la abre → col 2.
        let c1 = &cols[1];
        m.update_hover(c1.x + 10, row_y(c1.y, 1));
        assert_eq!(m.path, vec![1, 1]);
        assert_eq!(m.columns().len(), 3);
    }

    #[test]
    fn click_en_hoja_lanza_su_comando() {
        let mut m = menu();
        assert_eq!(
            m.click(150, row_y(100, 0)),
            ClickResult::Launch("kitty".into())
        );
    }

    #[test]
    fn click_en_submenu_lo_abre_y_sigue() {
        let mut m = menu();
        assert_eq!(m.click(150, row_y(100, 1)), ClickResult::Stay);
        assert_eq!(m.path, vec![1]);
    }

    #[test]
    fn click_fuera_cierra() {
        let mut m = menu();
        assert_eq!(m.click(5, 5), ClickResult::Close);
    }

    #[test]
    fn click_en_hoja_anidada_lanza() {
        let mut m = menu();
        m.update_hover(150, row_y(100, 1)); // Apps
        let cols = m.columns();
        let c1 = &cols[1];
        // Fila 0 de la columna 1 = "Navegador" (hoja).
        assert_eq!(
            m.click(c1.x + 10, row_y(c1.y, 0)),
            ClickResult::Launch("firefox".into())
        );
    }

    #[test]
    fn render_marca_submenus_y_resaltado() {
        let mut m = menu();
        m.update_hover(150, row_y(100, 1)); // abre Apps
        let view = m.render(150, row_y(100, 1));
        assert_eq!(view.len(), 2);
        // Columna 0: fila "Apps" es submenú y está resaltada (en path).
        let apps = &view[0].rows[1];
        assert!(apps.submenu);
        assert!(apps.highlighted);
        // "Terminal" es hoja, no resaltada.
        assert!(!view[0].rows[0].submenu);
    }

    #[test]
    fn open_acota_la_primera_columna_a_la_salida() {
        let m = RootMenu::open(790, 595, tree(), 800, 600);
        let c0 = &m.columns()[0];
        assert!(c0.x + MENU_W <= 800);
        assert!(c0.y + RootMenu::height_for(c0.len) <= 600);
    }
}
