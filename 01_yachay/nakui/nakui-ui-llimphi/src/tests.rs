    //! Tests del shell. Los tests del backend impl viven en `backend.rs`.
    //! Los helpers puros (preview_value/short_uuid/short_hash) en
    //! `nahual-meta-runtime`.

    use super::*;
    use serde_json::json;

    /// E2E mínimo del WAL: armamos un log a mano con dos seeds, abrimos
    /// con `EventLog::open` + `replay_into`, y verificamos que el
    /// `MemoryStore` queda con esos records aplicados. Reproduce el
    /// flujo del startup de NakuiBackend.
    #[test]
    fn event_log_replay_restores_memory_store() {
        use nakui_core::event_log::{replay_into, EventLog, LogEntry};
        use nakui_core::store::{MemoryStore, Store};
        use uuid::Uuid;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);

        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        {
            let mut log = EventLog::open(&path).unwrap();
            log.append(LogEntry::Seed {
                seq: 0,
                entity: "customer".into(),
                id: id_a,
                data: json!({"name": "Acme"}),
                schema_hash: None,
            })
            .unwrap();
            log.append(LogEntry::Seed {
                seq: 1,
                entity: "customer".into(),
                id: id_b,
                data: json!({"name": "Globex"}),
                schema_hash: None,
            })
            .unwrap();
        }

        let log = EventLog::open(&path).unwrap();
        assert_eq!(log.next_seq(), 2);
        let mut store = MemoryStore::new();
        replay_into(&log, &mut store).unwrap();

        assert_eq!(store.load("customer", id_a), Some(json!({"name": "Acme"})));
        assert_eq!(
            store.load("customer", id_b),
            Some(json!({"name": "Globex"}))
        );

        let _ = std::fs::remove_file(&path);
    }

    /// El layout del grafo round-trippea por el sidecar JSON (claves
    /// estables `(module_id, morfismo)`), y un archivo ausente da mapa
    /// vacío.
    #[test]
    fn graph_layout_round_trips_through_sidecar() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);

        // Archivo ausente → vacío.
        assert!(load_graph_layout(&path).is_empty());

        let mut pos: BTreeMap<(String, String), (f32, f32)> = BTreeMap::new();
        pos.insert(("ventas".into(), "calcular_total".into()), (120.0, 40.0));
        pos.insert(("ventas".into(), "marcar_pagado".into()), (300.5, 180.25));
        save_graph_layout(&pos, &path);

        let loaded = load_graph_layout(&path);
        assert_eq!(loaded, pos);

        let _ = std::fs::remove_file(&path);
    }

    /// El seeder de demo siembra el `seed.json` del módulo `ventas`,
    /// resuelve las refs `@handle` a UUIDs reales y es idempotente.
    #[test]
    fn seed_demo_data_seeds_ventas_and_is_idempotent() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);

        let modules_dir = std::path::Path::new("examples/nakui-modules");
        let (modules, _) = load_ui_modules(modules_dir).unwrap();
        let (mut backend, _) = NakuiBackend::open(path.clone(), 1000, BTreeMap::new());

        // Primer sembrado: 9 clientes + 12 órdenes.
        let toast = seed_demo_data(&mut backend, &modules, modules_dir);
        assert!(toast.is_some(), "debió sembrar en el primer arranque");
        let customers = backend.list_records("Customer");
        let orders = backend.list_records("Order");
        assert_eq!(customers.len(), 9);
        assert_eq!(orders.len(), 12);

        // Las refs `@handle` se resolvieron a UUIDs reales de Customer.
        let customer_ids: std::collections::BTreeSet<String> = customers
            .iter()
            .map(|(id, _)| id.to_string())
            .collect();
        for (_, ord) in &orders {
            let cust = ord.get("customer").and_then(Value::as_str).unwrap();
            assert!(
                customer_ids.contains(cust),
                "la orden referencia un Customer inexistente: {cust}"
            );
        }

        // Segundo sembrado: idempotente (entities no vacías → no toca nada).
        let again = seed_demo_data(&mut backend, &modules, modules_dir);
        assert!(again.is_none(), "no debió re-sembrar entities ya pobladas");
        assert_eq!(backend.list_records("Customer").len(), 9);
        assert_eq!(backend.list_records("Order").len(), 12);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(crate::backend::snapshot_path_for(&path));
    }

    /// Los KPIs de la ficha (`DetailMetric`) se scopean a los records
    /// relacionados: ACME tiene 2 órdenes (1200 + 800, ambas pagadas).
    #[test]
    fn detail_metric_scopes_to_related_records() {
        use nahual_meta_schema::{CardFilter, FilterOp, Metric};

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);

        let modules_dir = std::path::Path::new("examples/nakui-modules");
        let (modules, _) = load_ui_modules(modules_dir).unwrap();
        let (mut backend, _) = NakuiBackend::open(path.clone(), 1000, BTreeMap::new());
        seed_demo_data(&mut backend, &modules, modules_dir);

        let acme = backend
            .list_records("Customer")
            .into_iter()
            .find(|(_, v)| v.get("name").and_then(Value::as_str) == Some("ACME Corp"))
            .map(|(id, _)| id)
            .unwrap();

        let dm = |metric, filter| DetailMetric {
            label: "x".into(),
            entity: "Order".into(),
            via_field: "customer".into(),
            metric,
            filter,
            format: ValueFormat::default(),
        };

        assert_eq!(
            compute_detail_metric(&backend, &dm(Metric::Count, None), acme),
            MetricResult::Scalar(2.0)
        );
        assert_eq!(
            compute_detail_metric(
                &backend,
                &dm(Metric::Sum { field: "monto".into() }, None),
                acme
            ),
            MetricResult::Scalar(2000.0)
        );
        // Cobrado (pagado=true) = mismas 2 órdenes.
        let pagado = CardFilter {
            field: "pagado".into(),
            op: FilterOp::Eq,
            value: Some("true".into()),
            min: None,
            max: None,
        };
        assert_eq!(
            compute_detail_metric(
                &backend,
                &dm(Metric::Sum { field: "monto".into() }, Some(pagado)),
                acme
            ),
            MetricResult::Scalar(2000.0)
        );
        assert_eq!(
            compute_detail_metric(
                &backend,
                &dm(Metric::Avg { field: "monto".into() }, None),
                acme
            ),
            MetricResult::Scalar(1000.0)
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(crate::backend::snapshot_path_for(&path));
    }

    /// Las claves crudas de un desglose se muestran con su label: un
    /// `Select` resuelve a su `label` declarado, un booleano a Sí/No.
    #[test]
    fn humanize_relabels_select_and_boolean_keys() {
        use nahual_meta_schema::Metric;

        let modules_dir = std::path::Path::new("examples/nakui-modules");
        let (modules, _) = load_ui_modules(modules_dir).unwrap();
        let ventas = modules.iter().find(|m| m.id == "ventas").unwrap();

        // Select: tier → labels declarados; booleano → Sí/No; texto → sin mapa.
        let tier = field_label_map(ventas, "Customer", "tier").unwrap();
        assert_eq!(tier.get("pro").map(String::as_str), Some("Pro"));
        assert_eq!(tier.get("enterprise").map(String::as_str), Some("Enterprise"));
        let pagado = field_label_map(ventas, "Order", "pagado").unwrap();
        assert_eq!(pagado.get("true").map(String::as_str), Some("Sí"));
        assert_eq!(pagado.get("false").map(String::as_str), Some("No"));
        assert!(field_label_map(ventas, "Customer", "name").is_none());

        let card = |metric, group_ref: Option<&str>, bucket| DashboardCard {
            label: "x".into(),
            entity: "Customer".into(),
            metric,
            filter: None,
            format: ValueFormat::default(),
            group_ref: group_ref.map(Into::into),
            chart: ChartKind::Bars,
            limit: None,
            bucket,
            cumulative: false,
        };

        // GroupBy de tier: claves crudas → labels.
        let mut r = MetricResult::Breakdown(vec![("pro".into(), 3), ("free".into(), 2)]);
        humanize_breakdown_labels(
            &mut r,
            ventas,
            &card(Metric::GroupBy { field: "tier".into() }, None, None),
        );
        assert_eq!(
            r,
            MetricResult::Breakdown(vec![("Pro".into(), 3), ("Free".into(), 2)])
        );

        // group_ref presente → NO humaniza la dimensión de grupo.
        let mut r2 = MetricResult::Breakdown(vec![("pro".into(), 3)]);
        humanize_breakdown_labels(
            &mut r2,
            ventas,
            &card(Metric::GroupBy { field: "tier".into() }, Some("Customer"), None),
        );
        assert_eq!(r2, MetricResult::Breakdown(vec![("pro".into(), 3)]));

        // SumBySeries: la dimensión de serie (pagado) se humaniza a Sí/No.
        let order_card = DashboardCard {
            label: "x".into(),
            entity: "Order".into(),
            metric: Metric::SumBySeries {
                group: "fecha".into(),
                series: "pagado".into(),
                value: "monto".into(),
            },
            filter: None,
            format: ValueFormat::default(),
            group_ref: None,
            chart: ChartKind::Line,
            limit: None,
            bucket: Some(nahual_meta_schema::DateBucket::Month),
            cumulative: false,
        };
        let mut r3 = MetricResult::MultiBreakdown {
            groups: vec!["2026-01".into()],
            series: vec![("true".into(), vec![100.0]), ("false".into(), vec![50.0])],
        };
        humanize_breakdown_labels(&mut r3, ventas, &order_card);
        assert_eq!(
            r3,
            MetricResult::MultiBreakdown {
                // bucket activo → groups (fechas) intactos.
                groups: vec!["2026-01".into()],
                series: vec![("Sí".into(), vec![100.0]), ("No".into(), vec![50.0])],
            }
        );
    }

    /// El drill-down por prefijo (series temporales) recorta la lista al
    /// bucket: "2026-02" trae sólo las órdenes de febrero.
    #[test]
    fn drill_prefix_filters_list_to_month() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);

        let modules_dir = std::path::Path::new("examples/nakui-modules");
        let (modules, _) = load_ui_modules(modules_dir).unwrap();
        let (mut backend, _) = NakuiBackend::open(path.clone(), 1000, BTreeMap::new());
        seed_demo_data(&mut backend, &modules, modules_dir);

        let lv = ListView {
            title: "Órdenes".into(),
            entity: "Order".into(),
            columns: Vec::new(),
            actions: Vec::new(),
            search_in: Vec::new(),
            row_detail: None,
        };
        let feb = DrillFilter {
            entity: "Order".into(),
            field: "fecha".into(),
            value: "2026-02".into(),
            label: "2026-02".into(),
            prefix: true,
        };
        let rows = list_filtered_sorted(&backend, &lv, "", &None, Some(&feb));
        assert_eq!(rows.len(), 4, "deberían ser las 4 órdenes de febrero");
        assert!(rows
            .iter()
            .all(|(_, v)| v.get("fecha").and_then(Value::as_str).unwrap().starts_with("2026-02")));

        // Sin prefijo, "2026-02" no matchea ninguna fecha completa.
        let exact = DrillFilter { prefix: false, ..feb.clone() };
        assert_eq!(
            list_filtered_sorted(&backend, &lv, "", &None, Some(&exact)).len(),
            0
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(crate::backend::snapshot_path_for(&path));
    }

    /// `build_form` en alta: AutoId se rellena con un UUID, default
    /// puebla el resto, sin record original.
    #[test]
    fn build_form_fresh_fills_autoid_and_defaults() {
        let fv = FormView {
            title: "Nuevo".into(),
            entity: "Customer".into(),
            fields: vec![
                FieldSpec {
                    name: "id".into(),
                    label: "Id".into(),
                    kind: FieldKind::AutoId,
                    default: None,
                    required: false,
                    help: None,
                    ref_entity: None,
                    options: Vec::new(),
                    section: None,
                    item_fields: Vec::new(),
                    delimiter: None,
                },
                FieldSpec {
                    name: "tier".into(),
                    label: "Tier".into(),
                    kind: FieldKind::Text,
                    default: Some("free".into()),
                    required: false,
                    help: None,
                    ref_entity: None,
                    options: Vec::new(),
                    section: None,
                    item_fields: Vec::new(),
                    delimiter: None,
                },
            ],
            on_submit: Action::SeedEntity {
                entity: "Customer".into(),
                next_view: Some("list".into()),
            },
        };
        let form = build_form(0, &fv, None);
        assert!(form.editing.is_none());
        // AutoId parseable como UUID.
        assert!(Uuid::parse_str(&form.fields[0].raw()).is_ok());
        assert_eq!(form.fields[1].raw(), "free");
    }

    /// `build_form` en edición: pre-rellena desde el record original.
    #[test]
    fn build_form_editing_prefills_from_record() {
        let fv = FormView {
            title: "Editar".into(),
            entity: "Customer".into(),
            fields: vec![FieldSpec {
                name: "name".into(),
                label: "Nombre".into(),
                kind: FieldKind::Text,
                default: None,
                required: true,
                help: None,
                ref_entity: None,
                options: Vec::new(),
                section: None,
                item_fields: Vec::new(),
                delimiter: None,
            }],
            on_submit: Action::SeedEntity {
                entity: "Customer".into(),
                next_view: None,
            },
        };
        let id = Uuid::new_v4();
        let form = build_form(0, &fv, Some((id, json!({"name": "Acme"}))));
        assert_eq!(form.editing, Some(id));
        assert_eq!(form.fields[0].raw(), "Acme");
    }

    /// El módulo demo (`examples/nakui-modules/ventas.json`) carga,
    /// valida y trae los Form views esperados — guarda el fixture que
    /// el binario abre por default.
    #[test]
    fn demo_module_loads_and_validates() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples")
            .join("nakui-modules");
        let (modules, skipped) = load_ui_modules(&dir).expect("el módulo demo carga");
        assert!(skipped.is_empty(), "no debería skipear cards: {skipped:?}");
        // Cinco demos: 'ventas' (meta-form completo), 'tesoro' (vista grafo),
        // 'punto_venta' (POS: meta-form + morfismos), 'contabilidad'
        // (partida doble: form que dispara el morfismo `asentar`) y
        // 'facturacion' (la factura asienta vía `emitir_factura`).
        assert_eq!(modules.len(), 5);
        // Facturación expone tablero/lista/detalle + grafo, y su form de
        // emisión dispara el morfismo `emitir_factura`.
        let fact = modules
            .iter()
            .find(|m| m.id == "facturacion")
            .expect("facturacion");
        assert!(matches!(fact.views.get("tablero"), Some(ModuleView::Dashboard(_))));
        assert!(matches!(fact.views.get("facturas_list"), Some(ModuleView::List(_))));
        assert!(matches!(fact.views.get("factura_detail"), Some(ModuleView::Detail(_))));
        match fact.views.get("emitir_form") {
            Some(ModuleView::Form(fv)) => assert!(
                matches!(&fv.on_submit, Action::Morphism { name, .. } if name == "emitir_factura"),
                "el form de emisión debe disparar el morfismo `emitir_factura`"
            ),
            other => panic!("emitir_form debería ser un Form, fue {other:?}"),
        }
        // El form `facturar_form` dispara el morfismo `facturar` y tiene un
        // campo `lineas` de kind Array con sus columnas (item_fields).
        match fact.views.get("facturar_form") {
            Some(ModuleView::Form(fv)) => {
                assert!(matches!(&fv.on_submit, Action::Morphism { name, .. } if name == "facturar"));
                let lineas = fv
                    .fields
                    .iter()
                    .find(|f| f.name == "lineas")
                    .expect("el form tiene el campo lineas");
                assert_eq!(lineas.kind, FieldKind::Array);
                assert!(
                    !lineas.item_fields.is_empty(),
                    "el Array debe declarar columnas (item_fields)"
                );
            }
            other => panic!("facturar_form debería ser un Form, fue {other:?}"),
        }
        // Contabilidad expone las cuatro clases de vista + grafo, y su form
        // de asiento dispara un morfismo (no un seed directo).
        let cont = modules
            .iter()
            .find(|m| m.id == "contabilidad")
            .expect("contabilidad");
        assert!(matches!(cont.views.get("tablero"), Some(ModuleView::Dashboard(_))));
        assert!(matches!(cont.views.get("cuentas_list"), Some(ModuleView::List(_))));
        assert!(matches!(cont.views.get("cuenta_detail"), Some(ModuleView::Detail(_))));
        assert!(matches!(cont.views.get("libro"), Some(ModuleView::Graph(_))));
        match cont.views.get("asentar_form") {
            Some(ModuleView::Form(fv)) => assert!(
                matches!(&fv.on_submit, Action::Morphism { name, .. } if name == "asentar"),
                "el form de asiento debe disparar el morfismo `asentar`"
            ),
            other => panic!("asentar_form debería ser un Form, fue {other:?}"),
        }
        let tesoro = modules.iter().find(|m| m.id == "tesoro").expect("tesoro");
        assert!(
            matches!(tesoro.views.get("flujo"), Some(ModuleView::Graph(_))),
            "tesoro expone la vista grafo 'flujo'"
        );
        // El POS carga, valida y expone su grafo de morfismos.
        let pos = modules
            .iter()
            .find(|m| m.id == "punto_venta")
            .expect("punto_venta");
        assert!(matches!(pos.views.get("flujo"), Some(ModuleView::Graph(_))));
        assert!(find_form_view(pos, "Producto").is_some());
        assert!(find_form_view(pos, "Venta").is_some());
        assert!(find_form_view(pos, "LineaVenta").is_some());
        let m = modules.iter().find(|m| m.id == "ventas").expect("ventas");
        // Tiene un Form para cada entity (customers + orders).
        assert!(find_form_view(m, "Customer").is_some());
        assert!(find_form_view(m, "Order").is_some());
        // Y las cuatro clases de vista están presentes.
        assert!(matches!(m.views.get("tablero"), Some(ModuleView::Dashboard(_))));
        assert!(matches!(
            m.views.get("customer_detail"),
            Some(ModuleView::Detail(_))
        ));
        // La lista de clientes enlaza la ficha vía row_detail.
        if let Some(ModuleView::List(lv)) = m.views.get("customers_list") {
            assert_eq!(lv.row_detail.as_deref(), Some("customer_detail"));
        } else {
            panic!("customers_list debería ser una List");
        }
        // El form de cliente arma un FormState con AutoId pre-rellenado.
        let fv = find_form_view(m, "Customer").unwrap();
        let form = build_form(0, fv, None);
        let id_field = form
            .fields
            .iter()
            .find(|f| f.spec.kind == FieldKind::AutoId)
            .expect("el form tiene un AutoId");
        assert!(Uuid::parse_str(&id_field.raw()).is_ok());
    }

    /// End-to-end UI→backend→kernel: el form `asentar` (un MORFISMO, no
    /// un seed directo) corre por el backend con el executor real del
    /// módulo contabilidad. Verifica que mueve débito (+) y crédito (−),
    /// persiste el Asiento, y —lo central— CONSERVA la balanza: Σ de
    /// todos los saldos sigue en cero porque el kernel exige débito =
    /// crédito. Es la partida doble probada desde la capa de UI.
    #[test]
    fn asentar_via_backend_conserva_la_balanza() {
        use std::collections::BTreeMap;
        use std::sync::Arc;

        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples")
            .join("nakui-modules");
        let (modules, _) = load_ui_modules(&dir).expect("módulos cargan");

        // Construir el executor del módulo contabilidad (espejo de main.rs).
        let cont = modules.iter().find(|m| m.id == "contabilidad").unwrap();
        let nakui_dir = dir
            .join(&cont.id)
            .join(cont.nakui_module_dir.as_ref().unwrap());
        let exec = Executor::load_module(&nakui_dir).expect("executor contabilidad");
        let mut executors: BTreeMap<String, Arc<Executor>> = BTreeMap::new();
        executors.insert("contabilidad".into(), Arc::new(exec));

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);
        let (mut backend, _) = NakuiBackend::open(path.clone(), 1000, executors);

        // Sembrar el plan de cuentas de apertura (Σ saldos = 0).
        seed_demo_data(&mut backend, &modules, &dir);

        let suma = |b: &NakuiBackend| -> i64 {
            b.list_records("Cuenta")
                .iter()
                .map(|(_, v)| v.get("saldo").and_then(Value::as_i64).unwrap_or(0))
                .sum()
        };
        assert_eq!(suma(&backend), 0, "el plan de apertura debe cuadrar en cero");

        // Ubicar Caja (1010, activo) y Ventas (4010, ingreso) por código.
        let by_codigo = |b: &NakuiBackend, cod: &str| -> Uuid {
            b.list_records("Cuenta")
                .into_iter()
                .find(|(_, v)| v.get("codigo").and_then(Value::as_str) == Some(cod))
                .map(|(id, _)| id)
                .expect("cuenta existe")
        };
        let caja = by_codigo(&backend, "1010");
        let ventas = by_codigo(&backend, "4010");

        // Asentar: debe Caja / haber Ventas por 1000 (un cobro al contado).
        let mut inputs = BTreeMap::new();
        inputs.insert("debito".to_string(), caja);
        inputs.insert("credito".to_string(), ventas);
        let asiento_id = Uuid::new_v4();
        backend
            .morphism(
                "contabilidad",
                "asentar",
                inputs,
                json!({
                    "monto": 1000_i64,
                    "glosa": "cobro al contado",
                    "fecha": "2026-06-28",
                    "diario": "ventas",
                    "asiento_id": asiento_id.to_string(),
                }),
            )
            .expect("asentar debe pasar");

        // Débito sube el activo; crédito baja el ingreso (deudor-normal).
        let saldo = |b: &NakuiBackend, id: Uuid| -> i64 {
            b.load_record("Cuenta", id)
                .and_then(|v| v.get("saldo").and_then(Value::as_i64))
                .unwrap()
        };
        assert_eq!(saldo(&backend, caja), 5000 + 1000);
        assert_eq!(saldo(&backend, ventas), 0 - 1000);

        // El Asiento quedó persistido con sus dos patas.
        let asiento = backend
            .load_record("Asiento", asiento_id)
            .expect("asiento persistido");
        assert_eq!(asiento.get("monto").and_then(Value::as_i64), Some(1000));
        assert_eq!(
            asiento.get("debito_id").and_then(Value::as_str),
            Some(caja.to_string().as_str())
        );
        assert_eq!(
            asiento.get("credito_id").and_then(Value::as_str),
            Some(ventas.to_string().as_str())
        );

        // Lo central: la balanza sigue cuadrada tras el asiento.
        assert_eq!(suma(&backend), 0, "el asiento conserva la balanza en cero");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(crate::backend::snapshot_path_for(&path));
    }

    /// End-to-end UI→backend→kernel del módulo facturación: el form
    /// `emitir_factura` corre por el backend con el executor real, sobre
    /// el plan de cuentas que siembra `contabilidad` (store compartida).
    /// Verifica que la factura calcula el IVA, persiste la Factura y
    /// CONSERVA la balanza (debe Clientes = haber Ventas + IVA).
    #[test]
    fn emitir_factura_via_backend_asienta_balanceado() {
        use std::collections::BTreeMap;
        use std::sync::Arc;

        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples")
            .join("nakui-modules");
        let (modules, _) = load_ui_modules(&dir).expect("módulos cargan");

        // Executors de los dos módulos con nakui_module_dir que tocamos.
        let mut executors: BTreeMap<String, Arc<Executor>> = BTreeMap::new();
        for id in ["contabilidad", "facturacion"] {
            let m = modules.iter().find(|m| m.id == id).unwrap();
            let nakui_dir = dir.join(&m.id).join(m.nakui_module_dir.as_ref().unwrap());
            let exec = Executor::load_module(&nakui_dir).expect("executor carga");
            executors.insert(id.to_string(), Arc::new(exec));
        }

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);
        let (mut backend, _) = NakuiBackend::open(path.clone(), 1000, executors);

        // Siembra el plan de cuentas (incluye IVA por pagar 2110).
        seed_demo_data(&mut backend, &modules, &dir);

        let by_codigo = |b: &NakuiBackend, cod: &str| -> Uuid {
            b.list_records("Cuenta")
                .into_iter()
                .find(|(_, v)| v.get("codigo").and_then(Value::as_str) == Some(cod))
                .map(|(id, _)| id)
                .expect("cuenta existe")
        };
        let clientes = by_codigo(&backend, "1100");
        let ventas = by_codigo(&backend, "4010");
        let iva = by_codigo(&backend, "2110");
        let saldo = |b: &NakuiBackend, id: Uuid| -> i64 {
            b.load_record("Cuenta", id)
                .and_then(|v| v.get("saldo").and_then(Value::as_i64))
                .unwrap()
        };
        let suma_libro = |b: &NakuiBackend| -> i64 {
            b.list_records("Cuenta")
                .iter()
                .map(|(_, v)| v.get("saldo").and_then(Value::as_i64).unwrap_or(0))
                .sum()
        };
        assert_eq!(suma_libro(&backend), 0, "el plan de apertura cuadra");
        let (c0, v0, i0) = (saldo(&backend, clientes), saldo(&backend, ventas), saldo(&backend, iva));

        // Emitir: neto 1000, IVA 18% → impuesto 180, total 1180.
        let mut inputs = BTreeMap::new();
        inputs.insert("clientes_cta".to_string(), clientes);
        inputs.insert("ventas_cta".to_string(), ventas);
        inputs.insert("iva_cta".to_string(), iva);
        let factura_id = Uuid::new_v4();
        backend
            .morphism(
                "facturacion",
                "emitir_factura",
                inputs,
                json!({
                    "cliente": "ACME S.A.",
                    "fecha": "2026-06-28",
                    "factura_id": factura_id.to_string(),
                    "neto": 1000_i64,
                    "tasa": 18_i64,
                }),
            )
            .expect("emitir factura debe pasar");

        assert_eq!(saldo(&backend, clientes), c0 + 1180);
        assert_eq!(saldo(&backend, ventas), v0 - 1000);
        assert_eq!(saldo(&backend, iva), i0 - 180);

        let f = backend.load_record("Factura", factura_id).expect("factura persistida");
        assert_eq!(f.get("total").and_then(Value::as_i64), Some(1180));
        assert_eq!(f.get("impuesto").and_then(Value::as_i64), Some(180));

        // El libro entero sigue cuadrado tras la factura.
        assert_eq!(suma_libro(&backend), 0, "la factura conserva la balanza");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(crate::backend::snapshot_path_for(&path));
    }

    /// End-to-end del FieldKind::Array: toma las columnas (item_fields)
    /// del form `facturar_form`, parsea un texto multilínea de líneas con
    /// `parse_array_value` (como haría el submit), y corre `facturar` por
    /// el backend. Verifica que las líneas se crean, el neto se suma y el
    /// libro queda cuadrado — el array de la UI llega entero al morfismo.
    #[test]
    fn facturar_con_array_de_lineas_via_backend() {
        use std::collections::BTreeMap;
        use std::sync::Arc;

        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples")
            .join("nakui-modules");
        let (modules, _) = load_ui_modules(&dir).expect("módulos cargan");

        let mut executors: BTreeMap<String, Arc<Executor>> = BTreeMap::new();
        for id in ["contabilidad", "facturacion"] {
            let m = modules.iter().find(|m| m.id == id).unwrap();
            let nakui_dir = dir.join(&m.id).join(m.nakui_module_dir.as_ref().unwrap());
            executors.insert(id.to_string(), Arc::new(Executor::load_module(&nakui_dir).unwrap()));
        }
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);
        let (mut backend, _) = NakuiBackend::open(path.clone(), 1000, executors);
        seed_demo_data(&mut backend, &modules, &dir);

        // Tomar las columnas declaradas en el form de facturación.
        let fact = modules.iter().find(|m| m.id == "facturacion").unwrap();
        let item_fields = match fact.views.get("facturar_form") {
            Some(ModuleView::Form(fv)) => fv
                .fields
                .iter()
                .find(|f| f.name == "lineas")
                .unwrap()
                .item_fields
                .clone(),
            _ => panic!("falta facturar_form"),
        };
        // El texto que tipearía el usuario en el textarea del Array.
        let raw = "Servicio de diseño | 2 | 500\nHosting anual | 1 | 300";
        let lineas = parse_array_value(raw, &item_fields, "|").expect("parsea líneas");
        assert_eq!(lineas.as_array().unwrap().len(), 2);

        let by_codigo = |b: &NakuiBackend, cod: &str| -> Uuid {
            b.list_records("Cuenta")
                .into_iter()
                .find(|(_, v)| v.get("codigo").and_then(Value::as_str) == Some(cod))
                .map(|(id, _)| id)
                .unwrap()
        };
        let mut inputs = BTreeMap::new();
        inputs.insert("clientes_cta".to_string(), by_codigo(&backend, "1100"));
        inputs.insert("ventas_cta".to_string(), by_codigo(&backend, "4010"));
        inputs.insert("iva_cta".to_string(), by_codigo(&backend, "2110"));
        let factura_id = Uuid::new_v4();
        backend
            .morphism(
                "facturacion",
                "facturar",
                inputs,
                json!({
                    "cliente": "ACME S.A.",
                    "fecha": "2026-06-28",
                    "factura_id": factura_id.to_string(),
                    "tasa": 18_i64,
                    "lineas": lineas,
                }),
            )
            .expect("facturar por el backend debe pasar");

        // neto = 2*500 + 1*300 = 1300; IVA 18% = 234; total 1534.
        let f = backend.load_record("Factura", factura_id).expect("factura");
        assert_eq!(f.get("neto").and_then(Value::as_i64), Some(1300));
        assert_eq!(f.get("total").and_then(Value::as_i64), Some(1534));
        assert_eq!(backend.list_records("LineaFactura").len(), 2, "dos líneas creadas");

        let suma: i64 = backend
            .list_records("Cuenta")
            .iter()
            .map(|(_, v)| v.get("saldo").and_then(Value::as_i64).unwrap_or(0))
            .sum();
        assert_eq!(suma, 0, "la factura con líneas conserva la balanza");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(crate::backend::snapshot_path_for(&path));
    }

    #[test]
    fn next_sort_cycles_asc_desc_off() {
        // Columna nueva → ascendente.
        assert_eq!(next_sort(None, "name"), Some(("name".into(), true)));
        // Misma columna asc → desc.
        assert_eq!(
            next_sort(Some(("name".into(), true)), "name"),
            Some(("name".into(), false))
        );
        // Misma columna desc → sin orden.
        assert_eq!(next_sort(Some(("name".into(), false)), "name"), None);
        // Otra columna → arranca ascendente.
        assert_eq!(
            next_sort(Some(("name".into(), false)), "tier"),
            Some(("tier".into(), true))
        );
    }

    #[test]
    fn lookup_field_navigates_nested_paths() {
        let v = json!({"name": "Acme", "address": {"city": "Lima"}});
        assert_eq!(lookup_field(&v, "name"), Some(&json!("Acme")));
        assert_eq!(lookup_field(&v, "address.city"), Some(&json!("Lima")));
        assert_eq!(lookup_field(&v, "address.zip"), None);
        assert_eq!(lookup_field(&v, "missing"), None);
    }

    /// `cell_display` aplica el `ValueFormat` de la columna (sin
    /// ref_entity, no toca el backend).
    #[test]
    fn cell_display_formats_currency() {
        use nahual_meta_schema::Column;
        let col = Column {
            field: "monto".into(),
            label: "Monto".into(),
            weight: 1.0,
            ref_entity: None,
            format: ValueFormat::Currency { symbol: "$".into() },
        };
        let v = json!(12000);
        // No necesita backend porque la columna no es ref_entity; el
        // path de formato es puro.
        let out = format_value(Some(&v), &col.format);
        assert_eq!(out, "$12,000");
    }

    #[test]
    fn value_to_raw_covers_scalar_kinds() {
        assert_eq!(value_to_raw(&json!("hola")), "hola");
        assert_eq!(value_to_raw(&json!(true)), "true");
        assert_eq!(value_to_raw(&json!(42)), "42");
        assert_eq!(value_to_raw(&Value::Null), "");
    }

    #[test]
    fn graph_cone_separates_downstream_and_upstream() {
        // Topología del demo `tesoro`:
        //   1→2 (Movimiento), 2→3, 2→4 (Caja.saldo), 3→4 (Asiento).
        // Nodo 0 (abrir_caja) queda aislado.
        let w = |from_node: NodeId, to_node: NodeId| Wire {
            from_node,
            from_output: 0,
            to_node,
            to_input: 0,
        };
        let wires = vec![w(1, 2), w(2, 3), w(2, 4), w(3, 4)];

        // Cono de aplicar_movimiento (2): afecta a 3 y 4; depende de 1.
        let (down, up) = graph_cone(2, &wires, 5);
        assert_eq!(down.into_iter().collect::<Vec<_>>(), vec![3, 4]);
        assert_eq!(up.into_iter().collect::<Vec<_>>(), vec![1]);

        // Cono de cerrar_periodo (4): hoja, depende de 1,2,3; no afecta a nadie.
        let (down, up) = graph_cone(4, &wires, 5);
        assert!(down.is_empty());
        assert_eq!(up.into_iter().collect::<Vec<_>>(), vec![1, 2, 3]);

        // Nodo aislado (0): cono vacío en ambas direcciones.
        let (down, up) = graph_cone(0, &wires, 5);
        assert!(down.is_empty() && up.is_empty());
    }

    /// La Caja cobra el ticket: siembra una Venta + una LineaVenta por
    /// ítem y descuenta el stock del Producto.
    #[test]
    fn caja_charge_creates_sale_and_decrements_stock() {
        use std::collections::BTreeMap;
        use std::sync::{Arc, Mutex};

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let (mut backend, _status) =
            NakuiBackend::open(tmp.path().to_path_buf(), 50, BTreeMap::new());

        // Producto con stock 10.
        let mut prod = serde_json::Map::new();
        prod.insert("nombre".into(), json!("Café"));
        prod.insert("precio".into(), json!(20));
        prod.insert("stock".into(), json!(10));
        let pid = backend.seed("Producto", prod).unwrap().id.unwrap();

        let backend = Arc::new(Mutex::new(backend));
        let cart = vec![crate::caja::CartLine {
            product_id: pid,
            name: "Café".into(),
            price: 20.0,
            qty: 3,
        }];

        let (ok, _toast) = crate::caja::charge_cart(&backend, &cart, "efectivo");
        assert!(ok, "cobrar debería tener éxito");

        let b = backend.lock().unwrap();
        assert_eq!(b.list_records("Venta").len(), 1, "creó la venta");
        assert_eq!(b.list_records("LineaVenta").len(), 1, "creó una línea");
        let stock = b
            .load_record("Producto", pid)
            .unwrap()
            .get("stock")
            .and_then(|v| v.as_f64())
            .unwrap();
        assert_eq!(stock, 7.0, "descontó 3 del stock");
    }
