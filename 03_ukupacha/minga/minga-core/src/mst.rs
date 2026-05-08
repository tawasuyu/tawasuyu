//! Merkle Search Tree (MST).
//!
//! Estructura B-árbol probabilística sobre hashes, en la que el "nivel" de
//! cada clave se deriva determinísticamente de su propio hash (cantidad de
//! nibbles cero al inicio). Eso da dos propiedades clave:
//!
//! * **Independencia del orden de inserción.** El conjunto `{a, b, c}`
//!   siempre produce el mismo árbol y el mismo `root_hash`, sin importar
//!   en qué orden se insertaron las claves.
//! * **Comparación logarítmica.** Dos repositorios pueden saber si tienen
//!   el mismo conjunto de hashes con un único byte (`root_hash`); y, si
//!   difieren, descender solo por las ramas con hashes distintos.
//!
//! Esta implementación es completa para insert/contains/iter y produce un
//! `root_hash` Merkle correcto. La operación de `diff` mínima (delta de
//! sincronización P2P) se construirá encima cuando exista `minga-p2p`.

use crate::cas::ContentHash;
use blake3::Hasher;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Resumen estructural de un nodo interno del MST: nivel al que viven
/// sus claves, las claves a ese nivel, y el hash de cada uno de sus
/// hijos (subárboles). Esto es lo que un peer transmite cuando otro le
/// pregunta por la forma de un subárbol durante una sincronización
/// recursiva: bandwidth proporcional a la divergencia, no al tamaño.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NodeProbe {
    pub level: u32,
    pub keys: Vec<ContentHash>,
    pub child_hashes: Vec<ContentHash>,
}

/// Hash canónico del subárbol vacío (el "neutro" del MST). Cualquier
/// peer puede computarlo localmente sin tocar la red, lo que permite
/// reconocer ramas vacías en el otro lado sin pedir un probe.
pub fn empty_subtree_hash() -> ContentHash {
    static H: OnceLock<ContentHash> = OnceLock::new();
    *H.get_or_init(|| {
        let mut h = Hasher::new();
        h.update(b"E");
        ContentHash(*h.finalize().as_bytes())
    })
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Mst {
    root: Subtree,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
enum Subtree {
    #[default]
    Empty,
    Node(Box<NodeData>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NodeData {
    level: u32,
    keys: Vec<ContentHash>,
    children: Vec<Subtree>,
}

/// Nivel determinístico de un hash: número de nibbles (4 bits) cero al
/// inicio. Distribución geométrica con base 16, lo que da árbol balanceado
/// en expectativa con profundidad logarítmica.
fn level_of(h: &ContentHash) -> u32 {
    let mut count = 0u32;
    for &b in &h.0 {
        if b == 0 {
            count += 2;
        } else if b < 0x10 {
            count += 1;
            break;
        } else {
            break;
        }
    }
    count
}

impl Mst {
    pub fn new() -> Self {
        Self { root: Subtree::Empty }
    }

    /// Inserta `h`. Devuelve `true` si era una clave nueva.
    pub fn insert(&mut self, h: ContentHash) -> bool {
        let l = level_of(&h);
        let root = std::mem::take(&mut self.root);
        let (new_root, inserted) = insert_in(root, h, l);
        self.root = new_root;
        inserted
    }

    pub fn contains(&self, h: &ContentHash) -> bool {
        contains_in(&self.root, h)
    }

    pub fn len(&self) -> usize {
        len_of(&self.root)
    }

    pub fn is_empty(&self) -> bool {
        matches!(self.root, Subtree::Empty)
    }

    /// Recorrido in-order: claves emitidas en orden ascendente por hash.
    pub fn iter(&self) -> Iter<'_> {
        let mut it = Iter { stack: Vec::new() };
        it.descend_left(&self.root);
        it
    }

    /// Hash Merkle del árbol completo. Dos MSTs con el mismo conjunto de
    /// claves tienen el mismo `root_hash`, sin importar orden de inserción.
    pub fn root_hash(&self) -> ContentHash {
        subtree_hash(&self.root)
    }

    /// Construye un índice `subtree_hash -> NodeProbe` cubriendo cada
    /// nodo interno del árbol. Sirve a un peer como tabla de respuestas
    /// instantáneas a `ProbeReq`s del otro lado: dado un hash que el
    /// peer recibió de nosotros (en un Hello o un ProbeRes previo),
    /// podemos reconstituir su `NodeProbe` en `O(1)`.
    pub fn build_probe_index(&self) -> HashMap<ContentHash, NodeProbe> {
        let mut idx = HashMap::new();
        index_subtree(&self.root, &mut idx);
        idx
    }

    /// Diferencia simétrica entre `self` y `other`. Devuelve las claves
    /// que están en `self` pero no en `other`, y viceversa.
    ///
    /// Aprovecha la estructura Merkle: cualquier subárbol cuya raíz
    /// hashee igual entre ambos lados se descarta sin descender. Cuando
    /// dos nodos comparten nivel y separadores, recurrimos en paralelo
    /// sobre sus hijos — cada par idéntico se poda por hash. Cuando la
    /// estructura diverge (niveles distintos o separadores distintos en
    /// el mismo nivel), enumeramos las claves de ambos y hacemos merge
    /// ordenado.
    ///
    /// El resultado siempre viene ordenado por hash ascendente, lo que
    /// permite a un peer P2P hacer streaming de los bloques que faltan
    /// en orden estable y deduplicar mientras los recibe.
    pub fn diff(&self, other: &Mst) -> MstDiff {
        let mut d = MstDiff::default();
        diff_subtrees(&self.root, &other.root, &mut d.only_in_self, &mut d.only_in_other);
        d
    }
}

/// Resultado de comparar dos MSTs. `is_empty()` ⇔ ambos representan el
/// mismo conjunto.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MstDiff {
    pub only_in_self: Vec<ContentHash>,
    pub only_in_other: Vec<ContentHash>,
}

impl MstDiff {
    pub fn is_empty(&self) -> bool {
        self.only_in_self.is_empty() && self.only_in_other.is_empty()
    }

    pub fn total(&self) -> usize {
        self.only_in_self.len() + self.only_in_other.len()
    }
}

fn contains_in(t: &Subtree, h: &ContentHash) -> bool {
    match t {
        Subtree::Empty => false,
        Subtree::Node(n) => match n.keys.binary_search(h) {
            Ok(_) => true,
            Err(i) => contains_in(&n.children[i], h),
        },
    }
}

fn len_of(t: &Subtree) -> usize {
    match t {
        Subtree::Empty => 0,
        Subtree::Node(n) => n.keys.len() + n.children.iter().map(len_of).sum::<usize>(),
    }
}

fn subtree_hash(t: &Subtree) -> ContentHash {
    let mut h = Hasher::new();
    match t {
        Subtree::Empty => {
            h.update(b"E");
        }
        Subtree::Node(n) => {
            h.update(b"N");
            h.update(&n.level.to_le_bytes());
            h.update(&(n.keys.len() as u64).to_le_bytes());
            for k in &n.keys {
                h.update(&k.0);
            }
            for c in &n.children {
                h.update(&subtree_hash(c).0);
            }
        }
    }
    ContentHash(*h.finalize().as_bytes())
}

/// Inserta `h` (de nivel `l`) en el subárbol `t`. Devuelve el nuevo
/// subárbol y si fue una inserción real (no duplicado).
fn insert_in(t: Subtree, h: ContentHash, l: u32) -> (Subtree, bool) {
    match t {
        Subtree::Empty => {
            let node = NodeData {
                level: l,
                keys: vec![h],
                children: vec![Subtree::Empty, Subtree::Empty],
            };
            (Subtree::Node(Box::new(node)), true)
        }
        Subtree::Node(boxed) => {
            let n = *boxed;
            if l > n.level {
                // Nueva clave de nivel mayor: parte el árbol actual y la
                // promueve a nueva raíz.
                let (left, right) = split_at(Subtree::Node(Box::new(n)), &h);
                let new_root = NodeData {
                    level: l,
                    keys: vec![h],
                    children: vec![left, right],
                };
                (Subtree::Node(Box::new(new_root)), true)
            } else if l == n.level {
                match n.keys.binary_search(&h) {
                    Ok(_) => (Subtree::Node(Box::new(n)), false),
                    Err(i) => {
                        let NodeData { level, mut keys, mut children } = n;
                        let middle = std::mem::replace(&mut children[i], Subtree::Empty);
                        let (left, right) = split_at(middle, &h);
                        keys.insert(i, h);
                        children[i] = left;
                        children.insert(i + 1, right);
                        (
                            Subtree::Node(Box::new(NodeData { level, keys, children })),
                            true,
                        )
                    }
                }
            } else {
                // l < n.level: la clave nueva pertenece a un subárbol bajo
                // el separador correspondiente.
                let i = match n.keys.binary_search(&h) {
                    Ok(_) => unreachable!(
                        "colisión: clave de nivel inferior coincide con separador de nivel superior"
                    ),
                    Err(i) => i,
                };
                let NodeData { level, keys, mut children } = n;
                let child = std::mem::replace(&mut children[i], Subtree::Empty);
                let (new_child, inserted) = insert_in(child, h, l);
                children[i] = new_child;
                (
                    Subtree::Node(Box::new(NodeData { level, keys, children })),
                    inserted,
                )
            }
        }
    }
}

/// Parte `t` en (claves < pivot, claves > pivot). Pre-condición: el nivel
/// de cada subárbol involucrado es estrictamente menor que el del pivot
/// (que vive arriba). El pivot mismo no aparece en el resultado.
fn split_at(t: Subtree, pivot: &ContentHash) -> (Subtree, Subtree) {
    match t {
        Subtree::Empty => (Subtree::Empty, Subtree::Empty),
        Subtree::Node(boxed) => {
            let n = *boxed;
            let i = match n.keys.binary_search(pivot) {
                Ok(_) => unreachable!("pivot coincide con clave de nivel inferior"),
                Err(i) => i,
            };
            let NodeData { level, keys, children } = n;

            let mut left_keys = keys.clone();
            left_keys.truncate(i);
            let mut right_keys = keys;
            right_keys.drain(..i);

            let mut left_children: Vec<Subtree> = Vec::with_capacity(i + 1);
            let mut right_children: Vec<Subtree> = Vec::with_capacity(level as usize + 1);

            let mut iter = children.into_iter();
            for _ in 0..i {
                left_children.push(iter.next().expect("invariante: children > i"));
            }
            let middle = iter.next().expect("invariante: existe children[i]");
            let (l_mid, r_mid) = split_at(middle, pivot);
            left_children.push(l_mid);
            right_children.push(r_mid);
            for c in iter {
                right_children.push(c);
            }

            let left = if left_keys.is_empty() {
                left_children.pop().unwrap_or(Subtree::Empty)
            } else {
                Subtree::Node(Box::new(NodeData {
                    level,
                    keys: left_keys,
                    children: left_children,
                }))
            };
            let right = if right_keys.is_empty() {
                right_children.pop().unwrap_or(Subtree::Empty)
            } else {
                Subtree::Node(Box::new(NodeData {
                    level,
                    keys: right_keys,
                    children: right_children,
                }))
            };
            (left, right)
        }
    }
}

fn index_subtree(t: &Subtree, idx: &mut HashMap<ContentHash, NodeProbe>) {
    if let Subtree::Node(n) = t {
        let child_hashes: Vec<ContentHash> = n.children.iter().map(subtree_hash).collect();
        let probe = NodeProbe {
            level: n.level,
            keys: n.keys.clone(),
            child_hashes,
        };
        idx.insert(subtree_hash(t), probe);
        for c in &n.children {
            index_subtree(c, idx);
        }
    }
}

fn diff_subtrees(
    t1: &Subtree,
    t2: &Subtree,
    only_in_1: &mut Vec<ContentHash>,
    only_in_2: &mut Vec<ContentHash>,
) {
    // Short-circuit por hash Merkle: si los dos subárboles colapsan al
    // mismo hash de 32 bytes, representan el mismo conjunto. Una sola
    // comparación poda toda la rama. Aplicado recursivamente, en árboles
    // mayormente iguales el coste es proporcional a la divergencia, no al
    // tamaño total.
    if subtree_hash(t1) == subtree_hash(t2) {
        return;
    }
    match (t1, t2) {
        (Subtree::Empty, _) => collect_all(t2, only_in_2),
        (_, Subtree::Empty) => collect_all(t1, only_in_1),
        (Subtree::Node(n1), Subtree::Node(n2)) => {
            if n1.level == n2.level && n1.keys == n2.keys {
                // Mismo nivel y mismos separadores: los hijos se alinean
                // posicionalmente. Recurrimos en paralelo — cada par
                // idéntico se podará en su llamada por el hash de Merkle.
                for (c1, c2) in n1.children.iter().zip(n2.children.iter()) {
                    diff_subtrees(c1, c2, only_in_1, only_in_2);
                }
            } else {
                // Estructura divergente. Enumeramos ambos lados ordenados
                // y hacemos merge. Correcto pero sin más poda Merkle: una
                // futura iteración con `split_at` por cada separador del
                // nivel mayor recuperaría la poda en el caso desalineado.
                let mut k1 = Vec::with_capacity(len_of(t1));
                let mut k2 = Vec::with_capacity(len_of(t2));
                collect_all(t1, &mut k1);
                collect_all(t2, &mut k2);
                merge_diff_sorted(&k1, &k2, only_in_1, only_in_2);
            }
        }
    }
}

fn collect_all(t: &Subtree, out: &mut Vec<ContentHash>) {
    if let Subtree::Node(n) = t {
        for i in 0..n.keys.len() {
            collect_all(&n.children[i], out);
            out.push(n.keys[i]);
        }
        collect_all(&n.children[n.keys.len()], out);
    }
}

fn merge_diff_sorted(
    a: &[ContentHash],
    b: &[ContentHash],
    only_a: &mut Vec<ContentHash>,
    only_b: &mut Vec<ContentHash>,
) {
    let mut i = 0;
    let mut j = 0;
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => {
                only_a.push(a[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                only_b.push(b[j]);
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
        }
    }
    only_a.extend_from_slice(&a[i..]);
    only_b.extend_from_slice(&b[j..]);
}

pub struct Iter<'a> {
    /// Cada frame es (nodo, próximo índice de clave a emitir). Cuando se
    /// pushea un frame, ya descendimos por su hijo izquierdo (children[0]).
    stack: Vec<(&'a NodeData, usize)>,
}

impl<'a> Iter<'a> {
    fn descend_left(&mut self, t: &'a Subtree) {
        let mut cur = t;
        while let Subtree::Node(n) = cur {
            self.stack.push((n.as_ref(), 0));
            cur = &n.children[0];
        }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a ContentHash;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let (node, ki) = {
                let top = self.stack.last()?;
                (top.0, top.1)
            };
            if ki < node.keys.len() {
                self.stack.last_mut().unwrap().1 = ki + 1;
                self.descend_left(&node.children[ki + 1]);
                return Some(&node.keys[ki]);
            } else {
                self.stack.pop();
            }
        }
    }
}
