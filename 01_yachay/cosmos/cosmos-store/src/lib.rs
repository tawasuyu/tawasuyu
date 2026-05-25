//! `cosmos_app-store` — persistencia SQLite del estudio astrológico.
//!
//! Una sola conexión `rusqlite` envuelta en `Arc<Mutex>` para que la app
//! GPUI la comparta entre threads sin pelearse con el ownership. La
//! migración inicial corre la primera vez que se abre un archivo nuevo
//! (idempotente vía `CREATE TABLE IF NOT EXISTS`).
//!
//! Patrón inspirado en `nahual_provider_sqlite::SqliteDataProvider` pero
//! con dominio propio (no extiende el `DataProvider` agnóstico — esa
//! integración viene en `cosmos_app-tree` que envuelve este store
//! detrás del trait de nahual).

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;

use cosmos_model::{
    Chart, ChartId, ChartKind, Contact, ContactId, Group, GroupId, ModuleState, StoredBirthData,
    StoredChartConfig,
};

const SCHEMA_VERSION: i32 = 1;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("schema downgrade: db is at v{found}, code expects v{expected}")]
    SchemaDowngrade { found: i32, expected: i32 },
    #[error("ulid decode: {0}")]
    UlidDecode(#[from] ulid::DecodeError),
    #[error("model invariant: {0}")]
    Model(#[from] cosmos_model::ModelError),
    #[error("not found: {0}")]
    NotFound(String),
}

pub type StoreResult<T> = Result<T, StoreError>;

/// Store backed by a single SQLite file.
///
/// Clone-able: comparte la misma conexión bajo el mutex. Útil para que
/// distintos widgets (tree, panel, canvas) compartan una vista
/// consistente sin pasar `&mut` por todos lados.
#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    /// Abre (o crea) un archivo SQLite y corre las migraciones.
    pub fn open(path: impl AsRef<Path>) -> StoreResult<Self> {
        let conn = Connection::open(path)?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.migrate()?;
        Ok(store)
    }

    /// Variante in-memory para tests.
    pub fn in_memory() -> StoreResult<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> StoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(MIGRATION_V1)?;

        let found: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if found > SCHEMA_VERSION {
            return Err(StoreError::SchemaDowngrade {
                found,
                expected: SCHEMA_VERSION,
            });
        }
        if found < SCHEMA_VERSION {
            conn.execute(&format!("PRAGMA user_version = {}", SCHEMA_VERSION), [])?;
        }
        Ok(())
    }

    // -----------------------------------------------------------------
    // Groups
    // -----------------------------------------------------------------

    pub fn create_group(
        &self,
        parent_id: Option<GroupId>,
        name: &str,
        description: Option<&str>,
    ) -> StoreResult<Group> {
        let group = Group {
            id: GroupId::new(),
            parent_id,
            name: name.into(),
            description: description.map(String::from),
            created_at_ms: now_ms(),
            sort_order: 0,
        };
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO groups (id, parent_id, name, description, created_at_ms, sort_order) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                group.id.to_string(),
                group.parent_id.map(|g| g.to_string()),
                group.name,
                group.description,
                group.created_at_ms,
                group.sort_order,
            ],
        )?;
        Ok(group)
    }

    pub fn list_groups(&self, parent_id: Option<GroupId>) -> StoreResult<Vec<Group>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, parent_id, name, description, created_at_ms, sort_order \
             FROM groups WHERE parent_id IS ?1 \
             ORDER BY sort_order ASC, name COLLATE NOCASE ASC",
        )?;
        let parent_str = parent_id.map(|g| g.to_string());
        let rows = stmt.query_map(params![parent_str], row_to_group)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn delete_group(&self, id: GroupId) -> StoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM groups WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }

    pub fn rename_group(&self, id: GroupId, name: &str) -> StoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE groups SET name = ?2 WHERE id = ?1",
            params![id.to_string(), name],
        )?;
        Ok(())
    }

    /// Cambia el `parent_id` de un Group. Pasar `None` para mover a raíz.
    /// **No** valida ciclos — el caller debe garantizar que el nuevo
    /// padre no sea descendiente del que mueve (sino la DB queda con un
    /// ciclo que el list_groups no rompe pero hace al CTE infinito).
    pub fn move_group(&self, id: GroupId, new_parent: Option<GroupId>) -> StoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE groups SET parent_id = ?2 WHERE id = ?1",
            params![id.to_string(), new_parent.map(|g| g.to_string())],
        )?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Contacts
    // -----------------------------------------------------------------

    pub fn create_contact(
        &self,
        group_id: Option<GroupId>,
        name: &str,
        notes: Option<&str>,
    ) -> StoreResult<Contact> {
        let c = Contact {
            id: ContactId::new(),
            group_id,
            name: name.into(),
            notes: notes.map(String::from),
            created_at_ms: now_ms(),
        };
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO contacts (id, group_id, name, notes, created_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                c.id.to_string(),
                c.group_id.map(|g| g.to_string()),
                c.name,
                c.notes,
                c.created_at_ms,
            ],
        )?;
        Ok(c)
    }

    pub fn list_contacts(&self, group_id: Option<GroupId>) -> StoreResult<Vec<Contact>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, group_id, name, notes, created_at_ms \
             FROM contacts WHERE group_id IS ?1 \
             ORDER BY name COLLATE NOCASE ASC",
        )?;
        let g = group_id.map(|g| g.to_string());
        let rows = stmt.query_map(params![g], row_to_contact)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn delete_contact(&self, id: ContactId) -> StoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM contacts WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }

    pub fn rename_contact(&self, id: ContactId, name: &str) -> StoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE contacts SET name = ?2 WHERE id = ?1",
            params![id.to_string(), name],
        )?;
        Ok(())
    }

    pub fn move_contact(&self, id: ContactId, new_group: Option<GroupId>) -> StoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE contacts SET group_id = ?2 WHERE id = ?1",
            params![id.to_string(), new_group.map(|g| g.to_string())],
        )?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Charts
    // -----------------------------------------------------------------

    pub fn create_chart(
        &self,
        contact_id: ContactId,
        kind: ChartKind,
        label: &str,
        birth: &StoredBirthData,
        config: &StoredChartConfig,
        related_chart_id: Option<ChartId>,
    ) -> StoreResult<Chart> {
        let chart = Chart {
            id: ChartId::new(),
            contact_id,
            kind,
            label: label.into(),
            birth_data: birth.clone(),
            config: config.clone(),
            related_chart_id,
            created_at_ms: now_ms(),
        };
        chart.validate()?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO charts \
             (id, contact_id, kind, label, birth_data_json, config_json, \
              related_chart_id, created_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                chart.id.to_string(),
                chart.contact_id.to_string(),
                serde_json::to_string(&chart.kind)?,
                chart.label,
                serde_json::to_string(&chart.birth_data)?,
                serde_json::to_string(&chart.config)?,
                chart.related_chart_id.map(|c| c.to_string()),
                chart.created_at_ms,
            ],
        )?;
        Ok(chart)
    }

    pub fn list_charts(&self, contact_id: ContactId) -> StoreResult<Vec<Chart>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, contact_id, kind, label, birth_data_json, config_json, \
                    related_chart_id, created_at_ms \
             FROM charts WHERE contact_id = ?1 \
             ORDER BY created_at_ms ASC",
        )?;
        let rows = stmt.query_map(params![contact_id.to_string()], row_to_chart)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
            .and_then(|v| v.into_iter().collect::<StoreResult<Vec<_>>>())
    }

    /// Lista todas las cartas del DB ordenadas por label (case-insensitive).
    /// Pensado para pickers / selectores cross-contact (ej. elegir un
    /// partner de sinastría desde cualquier contacto).
    pub fn list_all_charts(&self) -> StoreResult<Vec<Chart>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, contact_id, kind, label, birth_data_json, config_json, \
                    related_chart_id, created_at_ms \
             FROM charts ORDER BY label COLLATE NOCASE ASC",
        )?;
        let rows = stmt.query_map([], row_to_chart)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    pub fn get_chart(&self, id: ChartId) -> StoreResult<Chart> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, contact_id, kind, label, birth_data_json, config_json, \
                    related_chart_id, created_at_ms \
             FROM charts WHERE id = ?1",
        )?;
        let chart = stmt
            .query_row(params![id.to_string()], row_to_chart)
            .optional()?;
        match chart {
            Some(Ok(c)) => Ok(c),
            Some(Err(e)) => Err(e),
            None => Err(StoreError::NotFound(format!("chart {}", id))),
        }
    }

    pub fn delete_chart(&self, id: ChartId) -> StoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM charts WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }

    pub fn rename_chart(&self, id: ChartId, label: &str) -> StoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE charts SET label = ?2 WHERE id = ?1",
            params![id.to_string(), label],
        )?;
        Ok(())
    }

    /// Reemplaza label + birth_data + config de una carta existente,
    /// preservando id / contact_id / related_chart_id / created_at_ms y
    /// el `module_state` asociado (no se borra). Usado por el editor de
    /// rectificación natal.
    pub fn update_chart(
        &self,
        id: ChartId,
        label: &str,
        birth: &StoredBirthData,
        config: &StoredChartConfig,
    ) -> StoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE charts SET label = ?2, birth_data_json = ?3, config_json = ?4 \
             WHERE id = ?1",
            params![
                id.to_string(),
                label,
                serde_json::to_string(birth)?,
                serde_json::to_string(config)?,
            ],
        )?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Module state
    // -----------------------------------------------------------------

    pub fn upsert_module_state(&self, state: &ModuleState) -> StoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO module_state (chart_id, module_id, enabled, config_json) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(chart_id, module_id) DO UPDATE SET \
                 enabled = excluded.enabled, \
                 config_json = excluded.config_json",
            params![
                state.chart_id.to_string(),
                state.module_id,
                state.enabled as i32,
                state.config.to_string(),
            ],
        )?;
        Ok(())
    }

    pub fn list_module_states(&self, chart_id: ChartId) -> StoreResult<Vec<ModuleState>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT chart_id, module_id, enabled, config_json \
             FROM module_state WHERE chart_id = ?1",
        )?;
        let rows = stmt.query_map(params![chart_id.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i32>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (chart_str, module_id, enabled, config_str) = r?;
            out.push(ModuleState {
                chart_id: chart_str
                    .parse()
                    .map_err(|e: ulid::DecodeError| StoreError::UlidDecode(e))?,
                module_id,
                enabled: enabled != 0,
                config: serde_json::from_str(&config_str).unwrap_or(serde_json::Value::Null),
            });
        }
        Ok(out)
    }

    // -----------------------------------------------------------------
    // Settings (key/value libre — layout, last-opened chart, etc.)
    // -----------------------------------------------------------------

    /// Lee un valor de la tabla `settings`. `None` si no existe.
    pub fn get_setting(&self, key: &str) -> StoreResult<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let val = conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(val)
    }

    /// Upsert un setting. El valor es texto libre — para JSON, el caller
    /// serializa antes de llamar.
    pub fn set_setting(&self, key: &str, value: &str) -> StoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Recursive descent: charts under a group/contact (para thumbnails)
    // -----------------------------------------------------------------

    /// Devuelve todas las cartas que descienden de un Group (incluyendo
    /// los Contacts de sub-groups recursivamente).
    pub fn charts_under_group(&self, root: GroupId) -> StoreResult<Vec<Chart>> {
        let conn = self.conn.lock().unwrap();
        // CTE recursivo para listar todos los descendientes del group.
        let mut stmt = conn.prepare(
            "WITH RECURSIVE descendants(id) AS ( \
                 SELECT ?1 \
                 UNION ALL \
                 SELECT g.id FROM groups g JOIN descendants d ON g.parent_id = d.id \
             ) \
             SELECT c.id, c.contact_id, c.kind, c.label, c.birth_data_json, c.config_json, \
                    c.related_chart_id, c.created_at_ms \
             FROM charts c \
             JOIN contacts ct ON ct.id = c.contact_id \
             WHERE ct.group_id IN descendants \
             ORDER BY c.created_at_ms ASC",
        )?;
        let rows = stmt.query_map(params![root.to_string()], row_to_chart)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }
}

// =====================================================================
// SQL schema
// =====================================================================

const MIGRATION_V1: &str = r#"
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS groups (
    id              TEXT PRIMARY KEY,
    parent_id       TEXT,
    name            TEXT NOT NULL,
    description     TEXT,
    created_at_ms   INTEGER NOT NULL,
    sort_order      INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY(parent_id) REFERENCES groups(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_groups_parent ON groups(parent_id);

CREATE TABLE IF NOT EXISTS contacts (
    id              TEXT PRIMARY KEY,
    group_id        TEXT,
    name            TEXT NOT NULL,
    notes           TEXT,
    created_at_ms   INTEGER NOT NULL,
    FOREIGN KEY(group_id) REFERENCES groups(id) ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS idx_contacts_group ON contacts(group_id);

CREATE TABLE IF NOT EXISTS charts (
    id                  TEXT PRIMARY KEY,
    contact_id          TEXT NOT NULL,
    kind                TEXT NOT NULL,
    label               TEXT NOT NULL,
    birth_data_json     TEXT NOT NULL,
    config_json         TEXT NOT NULL,
    related_chart_id    TEXT,
    created_at_ms       INTEGER NOT NULL,
    FOREIGN KEY(contact_id) REFERENCES contacts(id) ON DELETE CASCADE,
    FOREIGN KEY(related_chart_id) REFERENCES charts(id) ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS idx_charts_contact ON charts(contact_id);

CREATE TABLE IF NOT EXISTS module_state (
    chart_id    TEXT NOT NULL,
    module_id   TEXT NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 0,
    config_json TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY(chart_id, module_id),
    FOREIGN KEY(chart_id) REFERENCES charts(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

// =====================================================================
// Row decoders
// =====================================================================

fn row_to_group(row: &rusqlite::Row<'_>) -> rusqlite::Result<Group> {
    let id_str: String = row.get(0)?;
    let parent_id_str: Option<String> = row.get(1)?;
    Ok(Group {
        id: id_str
            .parse()
            .map_err(|e: ulid::DecodeError| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
        parent_id: match parent_id_str {
            Some(s) => Some(s.parse().map_err(|e: ulid::DecodeError| {
                rusqlite::Error::ToSqlConversionFailure(Box::new(e))
            })?),
            None => None,
        },
        name: row.get(2)?,
        description: row.get(3)?,
        created_at_ms: row.get(4)?,
        sort_order: row.get(5)?,
    })
}

fn row_to_contact(row: &rusqlite::Row<'_>) -> rusqlite::Result<Contact> {
    let id_str: String = row.get(0)?;
    let group_str: Option<String> = row.get(1)?;
    Ok(Contact {
        id: id_str
            .parse()
            .map_err(|e: ulid::DecodeError| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
        group_id: match group_str {
            Some(s) => Some(s.parse().map_err(|e: ulid::DecodeError| {
                rusqlite::Error::ToSqlConversionFailure(Box::new(e))
            })?),
            None => None,
        },
        name: row.get(2)?,
        notes: row.get(3)?,
        created_at_ms: row.get(4)?,
    })
}

fn row_to_chart(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoreResult<Chart>> {
    // Doble-Result porque hay deserialización JSON adentro que rusqlite no
    // sabe modelar. El caller la aplana.
    let id_str: String = row.get(0)?;
    let contact_str: String = row.get(1)?;
    let kind_json: String = row.get(2)?;
    let label: String = row.get(3)?;
    let bd_json: String = row.get(4)?;
    let cfg_json: String = row.get(5)?;
    let related_str: Option<String> = row.get(6)?;
    let created_at_ms: i64 = row.get(7)?;

    Ok((|| -> StoreResult<Chart> {
        Ok(Chart {
            id: id_str.parse()?,
            contact_id: contact_str.parse()?,
            kind: serde_json::from_str(&kind_json)?,
            label,
            birth_data: serde_json::from_str(&bd_json)?,
            config: serde_json::from_str(&cfg_json)?,
            related_chart_id: match related_str {
                Some(s) => Some(s.parse()?),
                None => None,
            },
            created_at_ms,
        })
    })())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use cosmos_model::{ModuleState, StoredBirthData, StoredChartConfig};

    #[test]
    fn open_and_migrate() {
        let s = Store::in_memory().unwrap();
        let groups = s.list_groups(None).unwrap();
        assert!(groups.is_empty());
    }

    #[test]
    fn module_state_roundtrip() {
        let s = Store::in_memory().unwrap();
        let g = s.create_group(None, "Familia", None).unwrap();
        let c = s.create_contact(Some(g.id), "Sergio", None).unwrap();
        let chart = s
            .create_chart(
                c.id,
                ChartKind::Natal,
                "Natal",
                &StoredBirthData {
                    year: 1987,
                    month: 3,
                    day: 14,
                    hour: 5,
                    minute: 22,
                    second: 0.0,
                    tz_offset_minutes: -240,
                    latitude_deg: 10.4806,
                    longitude_deg: -66.9036,
                    altitude_m: 900.0,
                    time_certainty: Default::default(),
                    subject_name: None,
                    birthplace_label: None,
                },
                &StoredChartConfig::default(),
                None,
            )
            .unwrap();

        // Persistir dos módulos con configs distintos.
        let state1 = ModuleState {
            chart_id: chart.id,
            module_id: "transit".into(),
            enabled: true,
            config: serde_json::json!({}),
        };
        let state2 = ModuleState {
            chart_id: chart.id,
            module_id: "progression".into(),
            enabled: false,
            config: serde_json::json!({ "target_age_years": 42.5 }),
        };
        s.upsert_module_state(&state1).unwrap();
        s.upsert_module_state(&state2).unwrap();

        let loaded = s.list_module_states(chart.id).unwrap();
        assert_eq!(loaded.len(), 2);
        let by_id: std::collections::HashMap<_, _> =
            loaded.into_iter().map(|m| (m.module_id.clone(), m)).collect();
        assert_eq!(by_id["transit"].enabled, true);
        assert_eq!(by_id["progression"].enabled, false);
        assert_eq!(
            by_id["progression"]
                .config
                .get("target_age_years")
                .and_then(|v| v.as_f64()),
            Some(42.5)
        );

        // Upsert: cambiar enabled de transit a false.
        let state1_off = ModuleState {
            chart_id: chart.id,
            module_id: "transit".into(),
            enabled: false,
            config: serde_json::json!({}),
        };
        s.upsert_module_state(&state1_off).unwrap();
        let loaded = s.list_module_states(chart.id).unwrap();
        let by_id: std::collections::HashMap<_, _> =
            loaded.into_iter().map(|m| (m.module_id.clone(), m)).collect();
        assert_eq!(by_id["transit"].enabled, false);
    }

    #[test]
    fn settings_upsert_and_read() {
        let s = Store::in_memory().unwrap();
        assert_eq!(s.get_setting("layout.outer").unwrap(), None);
        s.set_setting("layout.outer", "4.0,1.0").unwrap();
        assert_eq!(
            s.get_setting("layout.outer").unwrap().as_deref(),
            Some("4.0,1.0")
        );
        // Upsert — el segundo set sobreescribe.
        s.set_setting("layout.outer", "3.5,1.5").unwrap();
        assert_eq!(
            s.get_setting("layout.outer").unwrap().as_deref(),
            Some("3.5,1.5")
        );
    }

    #[test]
    fn full_hierarchy_roundtrip() {
        let s = Store::in_memory().unwrap();
        let g = s.create_group(None, "Familia", None).unwrap();
        let c = s.create_contact(Some(g.id), "Sergio", None).unwrap();
        let chart = s
            .create_chart(
                c.id,
                ChartKind::Natal,
                "Natal",
                &StoredBirthData {
                    year: 1987,
                    month: 3,
                    day: 14,
                    hour: 5,
                    minute: 22,
                    second: 0.0,
                    tz_offset_minutes: -240,
                    latitude_deg: 10.4806,
                    longitude_deg: -66.9036,
                    altitude_m: 900.0,
                    time_certainty: Default::default(),
                    subject_name: Some("Sergio".into()),
                    birthplace_label: Some("Caracas".into()),
                },
                &StoredChartConfig::default(),
                None,
            )
            .unwrap();
        assert_eq!(s.get_chart(chart.id).unwrap().label, "Natal");

        let under = s.charts_under_group(g.id).unwrap();
        assert_eq!(under.len(), 1);
        assert_eq!(under[0].id, chart.id);
    }
}
