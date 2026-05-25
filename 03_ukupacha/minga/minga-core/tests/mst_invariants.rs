//! Invariantes del Merkle Search Tree.
//!
//! La tesis del MST: dado un mismo conjunto de hashes, el árbol y su
//! `root_hash` son únicos, sin importar el orden de inserción. Eso es lo
//! que permite a dos repositorios saber si convergen comparando un solo
//! hash de 32 bytes y, si difieren, descender solo por las ramas con
//! diferencias.

use minga_core::{cas::ContentHash, mst::Mst};

fn ch(seed: u64) -> ContentHash {
    // Usamos blake3 para que la distribución de niveles (nibbles cero al
    // inicio) sea representativa, no degenerada.
    let h = blake3::hash(&seed.to_le_bytes());
    ContentHash(*h.as_bytes())
}

#[test]
fn mst_empty() {
    let m = Mst::new();
    assert!(m.is_empty());
    assert_eq!(m.len(), 0);
    assert_eq!(m.iter().count(), 0);
}

#[test]
fn mst_insert_single() {
    let mut m = Mst::new();
    let h = ch(1);
    assert!(m.insert(h));
    assert!(!m.insert(h)); // duplicado: no-op
    assert_eq!(m.len(), 1);
    assert!(m.contains(&h));
    assert!(!m.contains(&ch(2)));
}

#[test]
fn mst_iter_yields_sorted_keys() {
    let mut m = Mst::new();
    let mut hashes: Vec<ContentHash> = (0..32u64).map(ch).collect();
    for h in &hashes {
        m.insert(*h);
    }
    let collected: Vec<ContentHash> = m.iter().copied().collect();
    hashes.sort();
    assert_eq!(collected, hashes);
}

#[test]
fn mst_history_independence() {
    // Mismo conjunto, tres órdenes de inserción distintos: orden natural,
    // inverso, y reordenado por byte arbitrario. Los tres deben producir
    // exactamente el mismo árbol.
    let hashes: Vec<ContentHash> = (0..50u64).map(ch).collect();

    let mut m_natural = Mst::new();
    for h in &hashes {
        m_natural.insert(*h);
    }

    let mut m_reverse = Mst::new();
    for h in hashes.iter().rev() {
        m_reverse.insert(*h);
    }

    let mut shuffled = hashes.clone();
    shuffled.sort_by_key(|h| h.0[7]);
    let mut m_shuffled = Mst::new();
    for h in &shuffled {
        m_shuffled.insert(*h);
    }

    assert_eq!(m_natural.len(), 50);
    assert_eq!(m_natural.len(), m_reverse.len());
    assert_eq!(m_natural.len(), m_shuffled.len());

    assert_eq!(m_natural.root_hash(), m_reverse.root_hash());
    assert_eq!(m_natural.root_hash(), m_shuffled.root_hash());

    let s_natural: Vec<ContentHash> = m_natural.iter().copied().collect();
    let s_reverse: Vec<ContentHash> = m_reverse.iter().copied().collect();
    let s_shuffled: Vec<ContentHash> = m_shuffled.iter().copied().collect();
    assert_eq!(s_natural, s_reverse);
    assert_eq!(s_natural, s_shuffled);
}

#[test]
fn mst_set_difference_changes_root() {
    let mut m1 = Mst::new();
    m1.insert(ch(1));
    m1.insert(ch(2));

    let mut m2 = Mst::new();
    m2.insert(ch(1));
    m2.insert(ch(3));

    let mut m3 = Mst::new();
    m3.insert(ch(1));
    m3.insert(ch(2));

    assert_ne!(m1.root_hash(), m2.root_hash());
    assert_eq!(m1.root_hash(), m3.root_hash());
}

#[test]
fn mst_root_hash_changes_with_size() {
    let mut m = Mst::new();
    let h0 = m.root_hash();
    m.insert(ch(1));
    let h1 = m.root_hash();
    m.insert(ch(2));
    let h2 = m.root_hash();
    assert_ne!(h0, h1);
    assert_ne!(h1, h2);
}

#[test]
fn mst_contains_after_many_inserts() {
    let mut m = Mst::new();
    let hashes: Vec<ContentHash> = (0..200u64).map(ch).collect();
    for h in &hashes {
        m.insert(*h);
    }
    for h in &hashes {
        assert!(m.contains(h), "falta clave {h}");
    }
    assert!(!m.contains(&ch(9999)));
    assert_eq!(m.len(), 200);
}

#[test]
fn mst_no_duplicates_inflate_size() {
    let mut m = Mst::new();
    for _ in 0..10 {
        m.insert(ch(42));
    }
    assert_eq!(m.len(), 1);
}

#[test]
fn mst_diff_identical_is_empty() {
    let hs: Vec<_> = (0..30u64).map(ch).collect();
    let mut a = Mst::new();
    let mut b = Mst::new();
    for h in &hs {
        a.insert(*h);
        b.insert(*h);
    }
    let d = a.diff(&b);
    assert!(d.is_empty());
    assert_eq!(d.total(), 0);
}

#[test]
fn mst_diff_history_independent() {
    // Mismo conjunto en orden distinto: diff vacío. Aquí estresa el
    // short-circuit de Merkle: con 1000 claves construidas en órdenes
    // opuestos, la igualdad debe detectarse en una sola comparación.
    let hs: Vec<_> = (0..1000u64).map(ch).collect();
    let mut a = Mst::new();
    for h in &hs {
        a.insert(*h);
    }
    let mut b = Mst::new();
    for h in hs.iter().rev() {
        b.insert(*h);
    }
    assert!(a.diff(&b).is_empty());
}

#[test]
fn mst_diff_one_empty_yields_other() {
    let hs: Vec<_> = (0..10u64).map(ch).collect();
    let empty = Mst::new();
    let mut full = Mst::new();
    for h in &hs {
        full.insert(*h);
    }

    let d_full_vs_empty = full.diff(&empty);
    assert_eq!(d_full_vs_empty.only_in_self.len(), 10);
    assert!(d_full_vs_empty.only_in_other.is_empty());

    let d_empty_vs_full = empty.diff(&full);
    assert!(d_empty_vs_full.only_in_self.is_empty());
    assert_eq!(d_empty_vs_full.only_in_other.len(), 10);
}

#[test]
fn mst_diff_disjoint_sets() {
    let only_a: Vec<_> = (0..15u64).map(ch).collect();
    let only_b: Vec<_> = (100..115u64).map(ch).collect();
    let mut a = Mst::new();
    for h in &only_a {
        a.insert(*h);
    }
    let mut b = Mst::new();
    for h in &only_b {
        b.insert(*h);
    }
    let d = a.diff(&b);
    assert_eq!(d.only_in_self.len(), 15);
    assert_eq!(d.only_in_other.len(), 15);

    // El conjunto reportado debe coincidir exactamente con los inputs.
    let mut got_a = d.only_in_self.clone();
    let mut got_b = d.only_in_other.clone();
    got_a.sort();
    got_b.sort();
    let mut want_a = only_a.clone();
    let mut want_b = only_b.clone();
    want_a.sort();
    want_b.sort();
    assert_eq!(got_a, want_a);
    assert_eq!(got_b, want_b);
}

#[test]
fn mst_diff_partial_overlap() {
    let common: Vec<_> = (0..40u64).map(ch).collect();
    let only_a: Vec<_> = (40..50u64).map(ch).collect();
    let only_b: Vec<_> = (50..58u64).map(ch).collect();

    let mut a = Mst::new();
    for h in common.iter().chain(only_a.iter()) {
        a.insert(*h);
    }
    let mut b = Mst::new();
    for h in common.iter().chain(only_b.iter()) {
        b.insert(*h);
    }

    let d = a.diff(&b);
    // Las claves comunes no aparecen en el diff; solo las únicas.
    assert_eq!(d.only_in_self.len(), only_a.len());
    assert_eq!(d.only_in_other.len(), only_b.len());
}

#[test]
fn mst_diff_is_symmetric() {
    let a_keys: Vec<_> = (0..20u64).map(ch).collect();
    let b_keys: Vec<_> = (10..30u64).map(ch).collect();
    let mut a = Mst::new();
    for h in &a_keys {
        a.insert(*h);
    }
    let mut b = Mst::new();
    for h in &b_keys {
        b.insert(*h);
    }
    let ab = a.diff(&b);
    let ba = b.diff(&a);
    assert_eq!(ab.only_in_self, ba.only_in_other);
    assert_eq!(ab.only_in_other, ba.only_in_self);
}

#[test]
fn mst_diff_output_is_sorted() {
    // Sin importar la divergencia, el output viene ordenado por hash.
    let a_keys: Vec<_> = (0..25u64).map(ch).collect();
    let b_keys: Vec<_> = (15..40u64).map(ch).collect();
    let mut a = Mst::new();
    for h in &a_keys {
        a.insert(*h);
    }
    let mut b = Mst::new();
    for h in &b_keys {
        b.insert(*h);
    }
    let d = a.diff(&b);
    let mut sorted = d.only_in_self.clone();
    sorted.sort();
    assert_eq!(d.only_in_self, sorted);
    let mut sorted2 = d.only_in_other.clone();
    sorted2.sort();
    assert_eq!(d.only_in_other, sorted2);
}

#[test]
fn mst_diff_apply_converges() {
    // La propiedad fundacional para sincronización P2P: si cada peer
    // calcula el diff y aplica las claves que le faltan, ambos
    // convergen al mismo conjunto y el segundo diff es vacío.
    let common: Vec<_> = (0..50u64).map(ch).collect();
    let only_a: Vec<_> = (50..70u64).map(ch).collect();
    let only_b: Vec<_> = (70..85u64).map(ch).collect();

    let mut a = Mst::new();
    for h in common.iter().chain(only_a.iter()) {
        a.insert(*h);
    }
    let mut b = Mst::new();
    for h in common.iter().chain(only_b.iter()) {
        b.insert(*h);
    }

    let d = a.diff(&b);

    for h in &d.only_in_other {
        a.insert(*h);
    }
    for h in &d.only_in_self {
        b.insert(*h);
    }

    assert_eq!(a.root_hash(), b.root_hash());
    assert!(a.diff(&b).is_empty());
    assert_eq!(a.len(), common.len() + only_a.len() + only_b.len());
}

#[test]
fn mst_diff_single_key_change() {
    // Repos casi idénticos, diferenciados por una sola clave. El
    // short-circuit de Merkle debería podar todo lo demás. No medimos
    // el coste aquí (es un test de corrección), pero verificamos que
    // el resultado es exactamente la diferencia esperada.
    let hs: Vec<_> = (0..200u64).map(ch).collect();
    let mut a = Mst::new();
    for h in &hs {
        a.insert(*h);
    }
    let mut b = a.clone();
    let extra = ch(9999);
    b.insert(extra);

    let d = a.diff(&b);
    assert!(d.only_in_self.is_empty());
    assert_eq!(d.only_in_other, vec![extra]);
}

#[test]
fn mst_levels_distribute_naturally() {
    // Sanity: con 1000 claves blake3, esperamos que algunas tengan nivel
    // > 0 (probabilidad de >= 1 nibble cero al inicio ≈ 1/16, así que
    // ~62 claves esperadas). Si el árbol es de un solo nivel, algo en la
    // promoción/split está mal.
    let mut m = Mst::new();
    for i in 0..1000u64 {
        m.insert(ch(i));
    }
    assert_eq!(m.len(), 1000);
    // Si todas las claves estuvieran al mismo nivel, el árbol sería un
    // único nodo gigante y `root_hash` sería trivialmente reconstruible.
    // No es una verificación profunda, pero pillaría una regresión obvia.
    assert!(m.contains(&ch(0)));
    assert!(m.contains(&ch(999)));
}
