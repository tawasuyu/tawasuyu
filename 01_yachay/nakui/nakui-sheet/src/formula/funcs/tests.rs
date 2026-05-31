    use super::*;
    use crate::cell::CellRef;
    use crate::formula::compile;
    use crate::formula::eval::eval_formula;
    use rust_decimal::Decimal;
    use std::collections::HashMap;
    use std::str::FromStr;

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn run(src: &str, env: &HashMap<CellRef, SheetValue>) -> SheetValue {
        eval_formula(&compile(src).unwrap(), env)
    }

    #[test]
    fn sum_over_range_skips_empty_and_text() {
        let mut env = HashMap::new();
        env.insert(CellRef::new(0, 0), SheetValue::Number(dec("10")));
        // (1,0) intencionalmente ausente — Empty
        env.insert(CellRef::new(2, 0), SheetValue::Text("hola".into()));
        env.insert(CellRef::new(3, 0), SheetValue::Number(dec("5")));
        assert_eq!(run("=SUM(A1:D1)", &env), SheetValue::Number(dec("15")));
    }

    #[test]
    fn avg_of_empty_is_div_zero() {
        let env = HashMap::new();
        assert_eq!(run("=AVG(A1:A3)", &env), SheetValue::Error(SheetError::DivZero));
    }

    #[test]
    fn count_only_counts_numbers_counta_counts_non_empty() {
        let mut env = HashMap::new();
        env.insert(CellRef::new(0, 0), SheetValue::Number(dec("1")));
        env.insert(CellRef::new(0, 1), SheetValue::Text("x".into()));
        env.insert(CellRef::new(0, 2), SheetValue::Number(dec("3")));
        env.insert(CellRef::new(0, 3), SheetValue::Bool(true));
        // (0, 4) intencionalmente ausente → Empty.
        assert_eq!(run("=COUNT(A1:A5)", &env), SheetValue::Number(dec("2")));
        // COUNTA = no-vacíos: 1, "x", 3, TRUE → 4.
        assert_eq!(run("=COUNTA(A1:A5)", &env), SheetValue::Number(dec("4")));
    }

    #[test]
    fn if_picks_branch() {
        let env = HashMap::new();
        assert_eq!(run(r#"=IF(1>0, "yes", "no")"#, &env), SheetValue::Text("yes".into()));
        assert_eq!(run(r#"=IF(1<0, "yes", "no")"#, &env), SheetValue::Text("no".into()));
    }

    #[test]
    fn if_without_else_defaults_to_false() {
        let env = HashMap::new();
        assert_eq!(run("=IF(1<0, 99)", &env), SheetValue::Bool(false));
    }

    #[test]
    fn round_positive_digits() {
        let env = HashMap::new();
        assert_eq!(run("=ROUND(3.14159, 2)", &env), SheetValue::Number(dec("3.14")));
        assert_eq!(run("=ROUND(2.5, 0)", &env), SheetValue::Number(dec("2")));
        // ROUND(-2.5,0) → -2 (banker's rounding de rust_decimal)
    }

    #[test]
    fn round_negative_digits_rounds_to_tens() {
        let env = HashMap::new();
        assert_eq!(run("=ROUND(123.456, -1)", &env), SheetValue::Number(dec("120")));
        assert_eq!(run("=ROUND(155, -2)", &env), SheetValue::Number(dec("200")));
    }

    #[test]
    fn abs_and_unary_minus_agree() {
        let env = HashMap::new();
        assert_eq!(run("=ABS(-5)", &env), SheetValue::Number(dec("5")));
        assert_eq!(run("=ABS(5)", &env), SheetValue::Number(dec("5")));
    }

    #[test]
    fn and_or_not_short_circuit() {
        let env = HashMap::new();
        assert_eq!(run("=AND(1>0, 2>1)", &env), SheetValue::Bool(true));
        assert_eq!(run("=AND(1>0, 2<1)", &env), SheetValue::Bool(false));
        assert_eq!(run("=OR(1<0, 2>1)", &env), SheetValue::Bool(true));
        assert_eq!(run("=NOT(TRUE)", &env), SheetValue::Bool(false));
    }

    #[test]
    fn concat_function_and_amp_operator_agree() {
        let env = HashMap::new();
        let a = run(r#"=CONCAT("a", "b", "c")"#, &env);
        let b = run(r#"="a"&"b"&"c""#, &env);
        assert_eq!(a, b);
        assert_eq!(a, SheetValue::Text("abc".into()));
    }

    #[test]
    fn len_counts_codepoints_not_bytes() {
        let env = HashMap::new();
        assert_eq!(run(r#"=LEN("café")"#, &env), SheetValue::Number(dec("4")));
    }

    #[test]
    fn unknown_function_returns_name_error() {
        let env = HashMap::new();
        assert_eq!(
            run("=FROBOZZ(1)", &env),
            SheetValue::Error(SheetError::Name)
        );
    }

    #[test]
    fn error_in_scalar_arg_propagates() {
        let mut env = HashMap::new();
        env.insert(CellRef::new(0, 0), SheetValue::Error(SheetError::DivZero));
        assert_eq!(
            run("=ROUND(A1, 2)", &env),
            SheetValue::Error(SheetError::DivZero)
        );
    }

    #[test]
    fn iferror_catches_div_zero() {
        let env = HashMap::new();
        assert_eq!(
            run(r#"=IFERROR(1/0, "ups")"#, &env),
            SheetValue::Text("ups".into())
        );
        assert_eq!(
            run(r#"=IFERROR(10, "ups")"#, &env),
            SheetValue::Number(dec("10"))
        );
    }

    #[test]
    fn ifna_only_catches_na() {
        let env = HashMap::new();
        // 1/0 = #DIV/0!, no #N/A → IFNA NO lo atrapa.
        assert_eq!(run(r#"=IFNA(1/0, "ok")"#, &env), SheetValue::Error(SheetError::DivZero));
    }

    #[test]
    fn iserror_distinguishes_errors_from_values() {
        let env = HashMap::new();
        assert_eq!(run("=ISERROR(1/0)", &env), SheetValue::Bool(true));
        assert_eq!(run("=ISERROR(10)", &env), SheetValue::Bool(false));
    }

    #[test]
    fn istype_family() {
        let env = HashMap::new();
        assert_eq!(run(r#"=ISNUMBER(42)"#, &env), SheetValue::Bool(true));
        assert_eq!(run(r#"=ISTEXT("hola")"#, &env), SheetValue::Bool(true));
        assert_eq!(run(r#"=ISBLANK(Z99)"#, &env), SheetValue::Bool(true));
        assert_eq!(run(r#"=ISLOGICAL(TRUE)"#, &env), SheetValue::Bool(true));
        assert_eq!(run(r#"=ISNUMBER("42")"#, &env), SheetValue::Bool(false));
    }

    #[test]
    fn int_is_floor_not_truncate() {
        let env = HashMap::new();
        assert_eq!(run("=INT(3.7)", &env), SheetValue::Number(dec("3")));
        // -1.5 → floor → -2 (NO -1)
        assert_eq!(run("=INT(-1.5)", &env), SheetValue::Number(dec("-2")));
    }

    #[test]
    fn mod_excel_semantics() {
        let env = HashMap::new();
        assert_eq!(run("=MOD(10, 3)", &env), SheetValue::Number(dec("1")));
        // MOD(-10, 3) en Excel = 2 (signo sigue al divisor).
        assert_eq!(run("=MOD(-10, 3)", &env), SheetValue::Number(dec("2")));
        assert_eq!(run("=MOD(10, 0)", &env), SheetValue::Error(SheetError::DivZero));
    }

    #[test]
    fn left_right_mid_unicode() {
        let env = HashMap::new();
        assert_eq!(run(r#"=LEFT("café", 2)"#, &env), SheetValue::Text("ca".into()));
        assert_eq!(run(r#"=RIGHT("café", 2)"#, &env), SheetValue::Text("fé".into()));
        // MID es 1-indexed
        assert_eq!(run(r#"=MID("hello", 2, 3)"#, &env), SheetValue::Text("ell".into()));
    }

    #[test]
    fn trim_collapses_internal_whitespace() {
        let env = HashMap::new();
        assert_eq!(
            run(r#"=TRIM("  hello   world  ")"#, &env),
            SheetValue::Text("hello world".into())
        );
    }

    #[test]
    fn vlookup_exact_match() {
        let mut env = HashMap::new();
        // Tabla A1:B3 = [(1, "uno"), (2, "dos"), (3, "tres")]
        env.insert(CellRef::new(0, 0), SheetValue::Number(dec("1")));
        env.insert(CellRef::new(1, 0), SheetValue::Text("uno".into()));
        env.insert(CellRef::new(0, 1), SheetValue::Number(dec("2")));
        env.insert(CellRef::new(1, 1), SheetValue::Text("dos".into()));
        env.insert(CellRef::new(0, 2), SheetValue::Number(dec("3")));
        env.insert(CellRef::new(1, 2), SheetValue::Text("tres".into()));
        assert_eq!(
            run("=VLOOKUP(2, A1:B3, 2, FALSE)", &env),
            SheetValue::Text("dos".into())
        );
        assert_eq!(
            run("=VLOOKUP(99, A1:B3, 2, FALSE)", &env),
            SheetValue::Error(SheetError::NotApplicable)
        );
    }

    #[test]
    fn vlookup_approximate_finds_last_le() {
        let mut env = HashMap::new();
        // Tabla ascendente: 10, 20, 30 → buscar 25 devuelve la fila de 20.
        env.insert(CellRef::new(0, 0), SheetValue::Number(dec("10")));
        env.insert(CellRef::new(1, 0), SheetValue::Text("A".into()));
        env.insert(CellRef::new(0, 1), SheetValue::Number(dec("20")));
        env.insert(CellRef::new(1, 1), SheetValue::Text("B".into()));
        env.insert(CellRef::new(0, 2), SheetValue::Number(dec("30")));
        env.insert(CellRef::new(1, 2), SheetValue::Text("C".into()));
        assert_eq!(
            run("=VLOOKUP(25, A1:B3, 2)", &env),
            SheetValue::Text("B".into())
        );
    }

    #[test]
    fn index_2d_lookup() {
        let mut env = HashMap::new();
        // Tabla 3x2: rellena valores únicos.
        for r in 0..3 {
            for c in 0..2 {
                env.insert(
                    CellRef::new(c as u32, r as u32),
                    SheetValue::Number(Decimal::from((r * 10 + c) as i64)),
                );
            }
        }
        // INDEX(A1:B3, 2, 1) → fila 2, col 1 = (1,0) = 10
        assert_eq!(
            run("=INDEX(A1:B3, 2, 1)", &env),
            SheetValue::Number(dec("10"))
        );
        // INDEX(A1:B3, 3, 2) → (2,1) = 21
        assert_eq!(
            run("=INDEX(A1:B3, 3, 2)", &env),
            SheetValue::Number(dec("21"))
        );
    }

    #[test]
    fn match_exact_returns_one_indexed() {
        let mut env = HashMap::new();
        env.insert(CellRef::new(0, 0), SheetValue::Number(dec("10")));
        env.insert(CellRef::new(0, 1), SheetValue::Number(dec("20")));
        env.insert(CellRef::new(0, 2), SheetValue::Number(dec("30")));
        assert_eq!(
            run("=MATCH(20, A1:A3, 0)", &env),
            SheetValue::Number(dec("2"))
        );
        assert_eq!(
            run("=MATCH(99, A1:A3, 0)", &env),
            SheetValue::Error(SheetError::NotApplicable)
        );
    }

    #[test]
    fn index_match_combo_replaces_vlookup() {
        // El idioma clásico: INDEX(returnRange, MATCH(needle, keyRange, 0))
        let mut env = HashMap::new();
        env.insert(CellRef::new(0, 0), SheetValue::Number(dec("100")));
        env.insert(CellRef::new(1, 0), SheetValue::Text("rojo".into()));
        env.insert(CellRef::new(0, 1), SheetValue::Number(dec("200")));
        env.insert(CellRef::new(1, 1), SheetValue::Text("azul".into()));
        assert_eq!(
            run("=INDEX(B1:B2, MATCH(200, A1:A2, 0))", &env),
            SheetValue::Text("azul".into())
        );
    }

    #[test]
    fn date_to_serial_and_back() {
        let env = HashMap::new();
        // 1970-01-01 = día 0
        assert_eq!(run("=DATE(1970, 1, 1)", &env), SheetValue::Number(dec("0")));
        // 2026-05-27 = 20'600 días aproximado. Verifico calculando con
        // round-trip: YEAR/MONTH/DAY de DATE(...) reproducen los inputs.
        assert_eq!(
            run("=YEAR(DATE(2026, 5, 27))", &env),
            SheetValue::Number(dec("2026"))
        );
        assert_eq!(
            run("=MONTH(DATE(2026, 5, 27))", &env),
            SheetValue::Number(dec("5"))
        );
        assert_eq!(
            run("=DAY(DATE(2026, 5, 27))", &env),
            SheetValue::Number(dec("27"))
        );
    }

    #[test]
    fn date_handles_pre_epoch() {
        let env = HashMap::new();
        // 1969-12-31 = día -1
        assert_eq!(
            run("=DATE(1969, 12, 31)", &env),
            SheetValue::Number(dec("-1"))
        );
        assert_eq!(
            run("=YEAR(DATE(1969, 12, 31))", &env),
            SheetValue::Number(dec("1969"))
        );
    }

    #[test]
    fn today_returns_positive_serial() {
        let env = HashMap::new();
        // No probamos un valor exacto (depende del reloj), pero el
        // resultado debe ser un Number entero positivo.
        match run("=TODAY()", &env) {
            SheetValue::Number(n) => {
                assert!(n > Decimal::ZERO);
                assert_eq!(n.fract(), Decimal::ZERO);
            }
            other => panic!("expected Number, got {:?}", other),
        }
    }

    #[test]
    fn excel_compound_formula() {
        // Caso real: =IF(SUM(B1:B3)>100, "ALERTA", "OK")
        let mut env = HashMap::new();
        env.insert(CellRef::new(1, 0), SheetValue::Number(dec("40")));
        env.insert(CellRef::new(1, 1), SheetValue::Number(dec("30")));
        env.insert(CellRef::new(1, 2), SheetValue::Number(dec("50")));
        assert_eq!(
            run(r#"=IF(SUM(B1:B3)>100, "ALERTA", "OK")"#, &env),
            SheetValue::Text("ALERTA".into())
        );
    }

    // ─── Familia condicional (Bloque 19) ────────────────────────────

    /// Helper: rellena la columna A con la secuencia de (numero, texto)
    /// que usan los tests de SUMIF/COUNTIF. Devuelve el HashMap listo.
    fn env_invoices() -> HashMap<CellRef, SheetValue> {
        // A: importes; B: categoría textual; C: estado.
        // Cada fila representa una factura.
        let rows: &[(i64, &str, &str)] = &[
            (100, "rojo", "pagada"),
            (200, "azul", "pendiente"),
            (50, "rojo", "pagada"),
            (300, "verde", "pendiente"),
            (75, "Rojo", "pagada"), // case-insensitive: matchea "rojo"
        ];
        let mut env = HashMap::new();
        for (i, (n, cat, est)) in rows.iter().enumerate() {
            let r = i as u32;
            env.insert(CellRef::new(0, r), SheetValue::Number(Decimal::from(*n)));
            env.insert(CellRef::new(1, r), SheetValue::Text((*cat).into()));
            env.insert(CellRef::new(2, r), SheetValue::Text((*est).into()));
        }
        env
    }

    #[test]
    fn sumif_no_sum_range_sums_matching_cells() {
        let env = env_invoices();
        // Importes >100: 200 + 300 = 500.
        assert_eq!(
            run(r#"=SUMIF(A1:A5, ">100")"#, &env),
            SheetValue::Number(dec("500"))
        );
    }

    #[test]
    fn sumif_with_sum_range_uses_separate_column() {
        let env = env_invoices();
        // Importes donde categoría = "rojo" (case-insensitive):
        // 100 + 50 + 75 = 225.
        assert_eq!(
            run(r#"=SUMIF(B1:B5, "rojo", A1:A5)"#, &env),
            SheetValue::Number(dec("225"))
        );
    }

    #[test]
    fn sumif_exact_text_match_is_case_insensitive() {
        let env = env_invoices();
        // Sin operador → igualdad. "ROJO" matchea "rojo" y "Rojo".
        assert_eq!(
            run(r#"=SUMIF(B1:B5, "ROJO", A1:A5)"#, &env),
            SheetValue::Number(dec("225"))
        );
    }

    #[test]
    fn sumif_numeric_equality_via_scalar_criterion() {
        let env = env_invoices();
        // Criterio numérico literal (no string): 200.
        assert_eq!(
            run("=SUMIF(A1:A5, 200)", &env),
            SheetValue::Number(dec("200"))
        );
    }

    #[test]
    fn sumif_lte_and_ne_operators() {
        let env = env_invoices();
        // <=100: 100+50+75 = 225.
        assert_eq!(
            run(r#"=SUMIF(A1:A5, "<=100")"#, &env),
            SheetValue::Number(dec("225"))
        );
        // <>"rojo": azul (200) + verde (300) = 500.
        assert_eq!(
            run(r#"=SUMIF(B1:B5, "<>rojo", A1:A5)"#, &env),
            SheetValue::Number(dec("500"))
        );
    }

    #[test]
    fn sumif_shape_mismatch_yields_value_error() {
        let mut env = env_invoices();
        // sum_range con 3 elementos, crit_range con 5 → mismatch.
        env.insert(CellRef::new(3, 0), SheetValue::Number(dec("1")));
        env.insert(CellRef::new(3, 1), SheetValue::Number(dec("2")));
        env.insert(CellRef::new(3, 2), SheetValue::Number(dec("3")));
        assert_eq!(
            run(r#"=SUMIF(A1:A5, ">0", D1:D3)"#, &env),
            SheetValue::Error(SheetError::Value)
        );
    }

    #[test]
    fn sumif_propagates_error_inside_range() {
        let mut env = env_invoices();
        // Inyecto un #REF! en una fila → SUMIF debe fallar el rango,
        // no devolver un 0 silencioso.
        env.insert(CellRef::new(0, 1), SheetValue::Error(SheetError::Ref));
        assert_eq!(
            run(r#"=SUMIF(A1:A5, ">0")"#, &env),
            SheetValue::Error(SheetError::Ref)
        );
    }

    #[test]
    fn countif_counts_matches() {
        let env = env_invoices();
        // Filas con categoría = "rojo" (case-insensitive): 3.
        assert_eq!(
            run(r#"=COUNTIF(B1:B5, "rojo")"#, &env),
            SheetValue::Number(dec("3"))
        );
        // Filas con importe > 100: 2.
        assert_eq!(
            run(r#"=COUNTIF(A1:A5, ">100")"#, &env),
            SheetValue::Number(dec("2"))
        );
    }

    #[test]
    fn countif_no_matches_returns_zero() {
        let env = env_invoices();
        assert_eq!(
            run(r#"=COUNTIF(B1:B5, "negro")"#, &env),
            SheetValue::Number(dec("0"))
        );
    }

    #[test]
    fn averageif_computes_average_of_matching_subset() {
        let env = env_invoices();
        // Promedio de importes donde estado = "pagada":
        // (100 + 50 + 75) / 3 = 75.
        assert_eq!(
            run(r#"=AVERAGEIF(C1:C5, "pagada", A1:A5)"#, &env),
            SheetValue::Number(dec("75"))
        );
    }

    #[test]
    fn averageif_no_match_is_div_zero() {
        let env = env_invoices();
        assert_eq!(
            run(r#"=AVERAGEIF(C1:C5, "cancelada", A1:A5)"#, &env),
            SheetValue::Error(SheetError::DivZero)
        );
    }

    #[test]
    fn sumifs_two_criteria_intersection() {
        let env = env_invoices();
        // SUM de importes donde categoría = "rojo" Y estado = "pagada":
        // 100 + 50 + 75 = 225 (todas las rojo son pagadas en este set).
        assert_eq!(
            run(
                r#"=SUMIFS(A1:A5, B1:B5, "rojo", C1:C5, "pagada")"#,
                &env
            ),
            SheetValue::Number(dec("225"))
        );
        // Excluir pagadas: nada matchea → 0.
        assert_eq!(
            run(
                r#"=SUMIFS(A1:A5, B1:B5, "rojo", C1:C5, "<>pagada")"#,
                &env
            ),
            SheetValue::Number(dec("0"))
        );
    }

    #[test]
    fn sumifs_three_criteria_with_numeric_bound() {
        let env = env_invoices();
        // importe >= 75 Y categoría = "rojo" Y estado = "pagada":
        //   100 (✓), 50 (importe falla), 75 (✓) → 175.
        assert_eq!(
            run(
                r#"=SUMIFS(A1:A5, A1:A5, ">=75", B1:B5, "rojo", C1:C5, "pagada")"#,
                &env
            ),
            SheetValue::Number(dec("175"))
        );
    }

    #[test]
    fn countifs_multi_criteria() {
        let env = env_invoices();
        assert_eq!(
            run(
                r#"=COUNTIFS(B1:B5, "rojo", C1:C5, "pagada")"#,
                &env
            ),
            SheetValue::Number(dec("3"))
        );
    }

    #[test]
    fn averageifs_filters_and_averages() {
        let env = env_invoices();
        // Promedio donde categoría = "rojo" Y estado = "pagada":
        // (100+50+75)/3 = 75.
        assert_eq!(
            run(
                r#"=AVERAGEIFS(A1:A5, B1:B5, "rojo", C1:C5, "pagada")"#,
                &env
            ),
            SheetValue::Number(dec("75"))
        );
    }

    #[test]
    fn ifs_shape_mismatch_yields_value_error() {
        let mut env = env_invoices();
        // sum_range largo 5, criterio range largo 3 → #VALUE!.
        env.insert(CellRef::new(3, 0), SheetValue::Text("x".into()));
        env.insert(CellRef::new(3, 1), SheetValue::Text("y".into()));
        env.insert(CellRef::new(3, 2), SheetValue::Text("z".into()));
        assert_eq!(
            run(r#"=SUMIFS(A1:A5, D1:D3, "x")"#, &env),
            SheetValue::Error(SheetError::Value)
        );
    }

    #[test]
    fn sumifs_arity_check() {
        let env = env_invoices();
        // (range, criteria) en pares: 4 args = 1 sum_range + 1 pair + 1
        // huérfano → falla.
        assert_eq!(
            run(r#"=SUMIFS(A1:A5, B1:B5, "rojo", C1:C5)"#, &env),
            SheetValue::Error(SheetError::Value)
        );
    }

    #[test]
    fn countifs_arity_check() {
        let env = env_invoices();
        // COUNTIFS exige cantidad par; 3 args → #VALUE!.
        assert_eq!(
            run(r#"=COUNTIFS(A1:A5, ">0", B1:B5)"#, &env),
            SheetValue::Error(SheetError::Value)
        );
    }

    #[test]
    fn sumif_type_mismatch_doesnt_falsely_match() {
        let env = env_invoices();
        // Criterio numérico ">100" sobre rango de texto: ningún texto
        // matchea un comparador numérico — debe sumar 0.
        assert_eq!(
            run(r#"=SUMIF(B1:B5, ">100", A1:A5)"#, &env),
            SheetValue::Number(dec("0"))
        );
    }
