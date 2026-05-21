//! `MetaBackend` trait — la frontera entre el widget metainterfaz
//! (nahual) y la implementación concreta de persistencia/ejecución
//! (nakui-core, Surreal, mocks para tests).
//!
//! El widget consume este trait; el binario lo implementa con su
//! stack particular. Esto es lo que hace que el widget sea reusable.
//!
//! Convenciones documentadas en el doc del trait abajo.

use std::collections::BTreeMap;

use serde_json::Value;
use uuid::Uuid;

/// Resultado uniforme de una operación de escritura del backend.
///
/// La UI lo usa para componer el toast: `id` para mostrar el
/// short_uuid, `changed` para diferenciar "actualizado X (3 campos)"
/// vs "sin cambios", `post_status` para concatenar mensajes
/// emitidos por hooks internos del backend (ej. "auto-compact:
/// snapshot @ seq 49") sin que la UI tenga que conocer el detalle.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriteOutcome {
    /// Id del record afectado. `Some` para seed/update/delete;
    /// `None` para morphism cuando afecta múltiples records.
    pub id: Option<Uuid>,
    /// Cantidad de cambios efectivos. `0` = no-op (edit que no
    /// modificó ningún campo, etc.).
    pub changed: usize,
    /// Mensaje de status opcional para concatenar al toast del op
    /// original con el separator estándar.
    pub post_status: Option<String>,
}

impl WriteOutcome {
    /// Constructor para no-op writes (edits sin cambios).
    pub fn no_change(id: Uuid) -> Self {
        Self {
            id: Some(id),
            changed: 0,
            post_status: None,
        }
    }
}

/// Backend que un widget de metainterfaz usa para leer y mutar
/// records. Decoupla el widget (nahual) de la implementación
/// concreta (nakui-core, Surreal, mock para tests).
///
/// # Convención sobre ids
///
/// `Uuid` canónico. Backends que internamente usan otros tipos
/// deben mapear via Uuid (hash determinista, wrapper, lo que sirva).
/// Esto evita generic associated types que complicarían el dispatch
/// en `cx.listener` de GPUI.
///
/// # Convención sobre validación
///
/// El backend ES la fuente de verdad sobre invariantes (KCL/Nickel
/// post-checks, conservación, etc.). El widget pre-valida shape
/// (nahual-meta-runtime: `parse_field_value`, `validate_entity_refs`)
/// pero el backend puede rebotar con `Err(...)` si su validación
/// adicional falla — el widget muestra el error al usuario.
///
/// # Convención sobre threading
///
/// `'static` (no `Send + Sync`): el widget vive en `Entity<MetaApp<B>>`
/// que requiere `'static`, pero los handlers son single-threaded en
/// el main UI thread de GPUI. Si en el futuro un backend necesita
/// `cx.spawn`, agregamos los marker traits.
///
/// # Convención sobre delta computation
///
/// El widget pre-computa `set` y `clear` con
/// [`crate::delta::compute_field_delta`] +
/// [`crate::delta::compute_clear_fields`] *antes* de llamar a
/// [`MetaBackend::update`]. El backend no recomputa: si recibe ambos
/// vacíos devuelve `changed = 0` sin escribir nada. Esto evita
/// double-roundtrip al store por el mismo dato.
pub trait MetaBackend: 'static {
    /// Snapshot ordenado de records de una entity.
    /// Orden estable (lexicográfico por id) para UI determinista.
    /// Vacío si no hay records.
    fn list_records(&self, entity: &str) -> Vec<(Uuid, Value)>;

    /// Lee un record por id. `None` si no existe.
    fn load_record(&self, entity: &str, id: Uuid) -> Option<Value>;

    /// Crea un record nuevo. El backend asigna el `Uuid`
    /// (devuelve en `WriteOutcome.id`). `changed = 1` siempre.
    fn seed(
        &mut self,
        entity: &str,
        data: serde_json::Map<String, Value>,
    ) -> Result<WriteOutcome, String>;

    /// Edita un record existente. Aplica `set` (overrides) y
    /// `clear` (key removal). `changed = set.len() + clear.len()`.
    /// Si ambos están vacíos (no-op edit), devuelve
    /// `WriteOutcome::no_change(id)` sin error y sin escribir al log.
    fn update(
        &mut self,
        entity: &str,
        id: Uuid,
        set: serde_json::Map<String, Value>,
        clear: Vec<String>,
    ) -> Result<WriteOutcome, String>;

    /// Borra un record. `changed = 1` si existía, error si no.
    fn delete(&mut self, entity: &str, id: Uuid) -> Result<WriteOutcome, String>;

    /// Ejecuta un morphism declarado por un módulo. El backend
    /// resuelve la implementación, valida, computa ops, las aplica.
    /// `changed = N ops aplicadas`.
    ///
    /// `module_id` ubica al módulo (el trait no asume estructura del
    /// manifest — el backend lo resuelve internamente).
    fn morphism(
        &mut self,
        module_id: &str,
        name: &str,
        inputs: BTreeMap<String, Uuid>,
        params: Value,
    ) -> Result<WriteOutcome, String>;
}

#[cfg(test)]
mod tests {
    //! Tests del trait via [`crate::testing::MockBackend`]. Verifican
    //! el contrato genérico (object-safety, semantica de seed/update/
    //! delete) sin atar a un backend concreto. Los tests del mock en
    //! sí (constructores, with_morphism, etc.) viven en
    //! `crate::testing::tests`.

    use super::*;
    use crate::testing::MockBackend;
    use serde_json::json;

    fn map_of(items: &[(&str, Value)]) -> serde_json::Map<String, Value> {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn seed_then_load_round_trip() {
        let mut b = MockBackend::new();
        let out = b
            .seed("Customer", map_of(&[("name", json!("Acme"))]))
            .unwrap();
        let id = out.id.expect("seed devuelve id");
        assert_eq!(out.changed, 1);
        assert!(out.post_status.is_none());

        let rec = b.load_record("Customer", id).unwrap();
        assert_eq!(rec.get("name"), Some(&json!("Acme")));
    }

    #[test]
    fn list_records_filters_by_entity_and_orders_stably() {
        let mut b = MockBackend::new();
        let _ = b.seed("A", map_of(&[("k", json!(1))])).unwrap();
        let _ = b.seed("B", map_of(&[("k", json!(2))])).unwrap();
        let _ = b.seed("A", map_of(&[("k", json!(3))])).unwrap();

        let a = b.list_records("A");
        assert_eq!(a.len(), 2);
        let b_only = b.list_records("B");
        assert_eq!(b_only.len(), 1);
        let none = b.list_records("Missing");
        assert!(none.is_empty());

        // Orden estable: re-llamadas devuelven mismo orden.
        let a_again = b.list_records("A");
        assert_eq!(
            a.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            a_again.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn update_with_set_changes_field() {
        let mut b = MockBackend::new();
        let id = b
            .seed(
                "Customer",
                map_of(&[("name", json!("Acme")), ("notes", json!("x"))]),
            )
            .unwrap()
            .id
            .unwrap();

        let out = b
            .update(
                "Customer",
                id,
                map_of(&[("name", json!("Acme S.A."))]),
                vec![],
            )
            .unwrap();
        assert_eq!(out.changed, 1);
        assert_eq!(out.id, Some(id));

        let rec = b.load_record("Customer", id).unwrap();
        assert_eq!(rec.get("name"), Some(&json!("Acme S.A.")));
        assert_eq!(rec.get("notes"), Some(&json!("x")), "notes intacto");
    }

    #[test]
    fn update_with_clear_removes_key() {
        let mut b = MockBackend::new();
        let id = b
            .seed(
                "Customer",
                map_of(&[("name", json!("Acme")), ("notes", json!("x"))]),
            )
            .unwrap()
            .id
            .unwrap();

        let out = b
            .update("Customer", id, serde_json::Map::new(), vec!["notes".into()])
            .unwrap();
        assert_eq!(out.changed, 1);

        let rec = b.load_record("Customer", id).unwrap();
        assert_eq!(rec.get("name"), Some(&json!("Acme")));
        assert!(rec.get("notes").is_none(), "notes debería estar borrado");
    }

    #[test]
    fn update_with_empty_set_and_clear_returns_no_change() {
        let mut b = MockBackend::new();
        let id = b
            .seed("Customer", map_of(&[("name", json!("Acme"))]))
            .unwrap()
            .id
            .unwrap();

        let out = b
            .update("Customer", id, serde_json::Map::new(), vec![])
            .unwrap();
        assert_eq!(out, WriteOutcome::no_change(id));
    }

    #[test]
    fn update_on_missing_record_errors() {
        let mut b = MockBackend::new();
        let id = Uuid::new_v4();
        let err = b
            .update("Customer", id, map_of(&[("x", json!(1))]), vec![])
            .unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn delete_removes_and_then_load_returns_none() {
        let mut b = MockBackend::new();
        let id = b
            .seed("Customer", map_of(&[("name", json!("Acme"))]))
            .unwrap()
            .id
            .unwrap();
        let out = b.delete("Customer", id).unwrap();
        assert_eq!(out.changed, 1);
        assert_eq!(out.id, Some(id));
        assert!(b.load_record("Customer", id).is_none());
    }

    #[test]
    fn delete_on_missing_record_errors() {
        let mut b = MockBackend::new();
        let id = Uuid::new_v4();
        assert!(b.delete("Customer", id).is_err());
    }

    /// Sanity: el trait acepta llamadas via `&mut dyn MetaBackend`
    /// (object-safety). Esto permite que el widget tenga
    /// `Box<dyn MetaBackend>` si el use case requiere borrado de
    /// tipo (vs. el path normal con `MetaApp<B: MetaBackend>`).
    #[test]
    fn trait_is_object_safe() {
        let mut b: Box<dyn MetaBackend> = Box::new(MockBackend::new());
        let _ = b.seed("X", map_of(&[("k", json!(1))])).unwrap();
        assert_eq!(b.list_records("X").len(), 1);
    }
}
