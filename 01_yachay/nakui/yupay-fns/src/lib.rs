//! `yupay-fns` — el catálogo de funciones de hoja sobre el lenguaje de
//! `yupay-core`. Implementa [`FuncDispatch`] vía [`Funcs`]; el evaluador de
//! `yupay-core` lo recibe por parámetro, así el lenguaje queda independiente
//! del catálogo (regla #1 del repo: split del motor > ~2000 LOC).
//!
//! **Bilingüe** (PLAN.md §6.ter): cada función tiene un nombre canónico
//! inglés (el de Excel: `SUM`, `VLOOKUP`…) y aliases en español y quechua que
//! [`canonical`] normaliza antes del dispatch. El usuario escribe `=SUMA(...)`
//! o `=SUM(...)` o `=YAPAY(...)` y todos rutean a la misma implementación.
//!
//! El lexer de `yupay-core` acepta identificadores Unicode con punto y acento,
//! así que los aliases Excel-es genuinos rutean tal cual: `SUMAR.SI`, `AÑO`,
//! `MÁXIMO` (y también sus formas ASCII `SUMARSI`/`ANIO`/`MAXIMO`). Verificado
//! por los tests de este crate (`=MÁXIMO(...)`, `=SUMAR.SI(...)`).
//!
//! El dispatch va por nombre UPPERCASE (el parser ya normaliza). Si el nombre
//! no existe devolvemos `#NAME?` — como Excel cuando uno teclea mal una
//! función. Cada función ignora celdas vacías al agregar (igual que SUM),
//! pero `COUNT` sólo cuenta numéricos; texto no-parseable da `#VALUE!` sólo en
//! contextos numéricos puros (las agregadas lo saltan, `1 + "abc"` sí cae).

use rust_decimal::Decimal;
use yupay_core::{FormulaArg, FuncDispatch, SheetError, SheetValue};

mod aggregate;
mod criteria;
mod datetime;
mod lookup;
mod scalar;
#[cfg(test)]
mod tests;

// Helpers compartidos (arity, scalar_*, flatten_numbers…) re-exportados
// pub(crate) para que cada submódulo los vea vía `use super::*`.
pub(crate) use aggregate::*;
pub(crate) use criteria::*;
pub(crate) use datetime::*;
pub(crate) use lookup::*;
pub(crate) use scalar::*;

/// Despachador concreto de funciones, el que `yupay-core` invoca al evaluar
/// un `Call`. Sin estado: una unidad de tipo. Construir `Funcs` y pasarlo a
/// `yupay_core::eval_formula` es todo lo que hace falta para tener el catálogo.
pub struct Funcs;

impl FuncDispatch for Funcs {
    fn call(&self, name: &str, args: &[FormulaArg]) -> SheetValue {
        dispatch(name, args)
    }
}

/// Traduce un alias es/qu al nombre canónico inglés. Los nombres ya en inglés
/// (y los desconocidos) pasan sin cambio — el `match` de [`dispatch`] decide
/// si existen. Entra en UPPERCASE (lo garantiza el lexer/parser).
pub fn canonical(name: &str) -> &str {
    // Los nombres Excel-es genuinos llevan punto (`SUMAR.SI`) y acentos
    // (`AÑO`, `MÁXIMO`); el lexer de yupay-core ya los acepta. Mantenemos
    // además variantes dot-free / sin-acento (`SUMARSI`, `ANIO`) como
    // tolerancia para quien las teclee. El quechua es semilla (YUPAY/YAPAY).
    match name {
        // --- Agregadas ---
        "SUMA" | "YAPAY" => "SUM",
        "PROMEDIO" => "AVERAGE",
        "MÍNIMO" | "MINIMO" => "MIN",
        "MÁXIMO" | "MAXIMO" => "MAX",
        "CONTAR" | "YUPAY" => "COUNT",
        "CONTARA" => "COUNTA",
        "SUMAR.SI" | "SUMARSI" => "SUMIF",
        "CONTAR.SI" | "CONTARSI" => "COUNTIF",
        "PROMEDIO.SI" | "PROMEDIOSI" => "AVERAGEIF",
        "SUMAR.SI.CONJUNTO" | "SUMARSICONJUNTO" => "SUMIFS",
        "CONTAR.SI.CONJUNTO" | "CONTARSICONJUNTO" => "COUNTIFS",
        "PROMEDIO.SI.CONJUNTO" | "PROMEDIOSICONJUNTO" => "AVERAGEIFS",
        // --- Escalares / numéricas ---
        "REDONDEAR" => "ROUND",
        "ENTERO" => "INT",
        "RESIDUO" => "MOD",
        // --- Lógicas ---
        "SI" => "IF",
        "SI.ERROR" | "SIERROR" => "IFERROR",
        "SI.ND" | "SIND" => "IFNA",
        "Y" => "AND",
        "O" => "OR",
        "NO" => "NOT",
        "ES.ERROR" | "ESERROR" => "ISERROR",
        "ES.NUMERO" | "ESNUMERO" => "ISNUMBER",
        "ES.TEXTO" | "ESTEXTO" => "ISTEXT",
        "ES.BLANCO" | "ESBLANCO" => "ISBLANK",
        "ES.LOGICO" | "ESLOGICO" => "ISLOGICAL",
        // --- Texto ---
        "CONCATENAR" => "CONCAT",
        "LARGO" => "LEN",
        "MAYUSC" => "UPPER",
        "MINUSC" => "LOWER",
        "IZQUIERDA" => "LEFT",
        "DERECHA" => "RIGHT",
        "EXTRAE" => "MID",
        "ESPACIOS" => "TRIM",
        // --- Búsqueda ---
        "BUSCARV" => "VLOOKUP",
        "ÍNDICE" | "INDICE" => "INDEX",
        "COINCIDIR" => "MATCH",
        // --- Fecha ---
        "FECHA" => "DATE",
        "HOY" => "TODAY",
        "AHORA" => "NOW",
        "AÑO" | "ANIO" => "YEAR",
        "MES" => "MONTH",
        "DÍA" | "DIA" => "DAY",
        "ALEATORIO" => "RAND",
        "ALEATORIO.ENTRE" | "ALEATORIOENTRE" => "RANDBETWEEN",
        // En inglés o desconocido: tal cual.
        other => other,
    }
}

pub fn dispatch(name: &str, args: &[FormulaArg]) -> SheetValue {
    let name = canonical(name);

    // Las funciones de información (`ISERROR`, `IFERROR`, `IFNA`) NO
    // deben propagar errores — su trabajo es justamente inspeccionar/
    // atrapar el error. Para el resto, errores en cualquier argumento
    // escalar se propagan antes de entrar.
    let propagates = !matches!(name, "ISERROR" | "IFERROR" | "IFNA");
    if propagates {
        for a in args {
            if let FormulaArg::Value(SheetValue::Error(e)) = a {
                return SheetValue::Error(e.clone());
            }
        }
    }

    match name {
        "SUM" => agg_sum(args),
        "AVG" | "AVERAGE" => agg_average(args),
        "MIN" => agg_min(args),
        "MAX" => agg_max(args),
        "COUNT" => agg_count(args),
        "COUNTA" => agg_counta(args),
        "SUMIF" => agg_sumif(args),
        "COUNTIF" => agg_countif(args),
        "AVERAGEIF" | "AVGIF" => agg_averageif(args),
        "SUMIFS" => agg_sumifs(args),
        "COUNTIFS" => agg_countifs(args),
        "AVERAGEIFS" | "AVGIFS" => agg_averageifs(args),
        "ROUND" => fn_round(args),
        "ABS" => fn_abs(args),
        "INT" => fn_int(args),
        "MOD" => fn_mod(args),
        "IF" => fn_if(args),
        "IFERROR" => fn_iferror(args),
        "IFNA" => fn_ifna(args),
        "AND" => fn_and(args),
        "OR" => fn_or(args),
        "NOT" => fn_not(args),
        "ISERROR" => fn_iserror(args),
        "ISNUMBER" => fn_istype(args, |v| matches!(v, SheetValue::Number(_))),
        "ISTEXT" => fn_istype(args, |v| matches!(v, SheetValue::Text(_))),
        "ISBLANK" => fn_istype(args, |v| matches!(v, SheetValue::Empty)),
        "ISLOGICAL" => fn_istype(args, |v| matches!(v, SheetValue::Bool(_))),
        "CONCAT" | "CONCATENATE" => fn_concat(args),
        "LEN" => fn_len(args),
        "UPPER" => fn_upper(args),
        "LOWER" => fn_lower(args),
        "LEFT" => fn_left(args),
        "RIGHT" => fn_right(args),
        "MID" => fn_mid(args),
        "TRIM" => fn_trim(args),
        "VLOOKUP" => fn_vlookup(args),
        "INDEX" => fn_index(args),
        "MATCH" => fn_match(args),
        "DATE" => fn_date(args),
        "TODAY" => fn_today(args),
        "NOW" => fn_now(args),
        "YEAR" => fn_year(args),
        "MONTH" => fn_month(args),
        "DAY" => fn_day(args),
        "RAND" => fn_rand(args),
        "RANDBETWEEN" => fn_randbetween(args),
        _ => SheetValue::Error(SheetError::Name),
    }
}

#[cfg(test)]
mod bilingue {
    use super::*;
    use rust_decimal::Decimal;
    use std::collections::HashMap;
    use yupay_core::{compile, eval_formula, CellRef};

    fn run(src: &str) -> SheetValue {
        let mut env: HashMap<CellRef, SheetValue> = HashMap::new();
        env.insert(CellRef::new(0, 0), SheetValue::Number(Decimal::from(10)));
        env.insert(CellRef::new(0, 1), SheetValue::Number(Decimal::from(20)));
        env.insert(CellRef::new(0, 2), SheetValue::Number(Decimal::from(30)));
        eval_formula(&compile(src).unwrap(), &env, &Funcs)
    }

    #[test]
    fn canonical_traduce_es_a_en() {
        assert_eq!(canonical("SUMA"), "SUM");
        assert_eq!(canonical("PROMEDIO"), "AVERAGE");
        assert_eq!(canonical("SI"), "IF");
        assert_eq!(canonical("BUSCARV"), "VLOOKUP");
        // En inglés o desconocido pasan sin cambio.
        assert_eq!(canonical("SUM"), "SUM");
        assert_eq!(canonical("NOEXISTE"), "NOEXISTE");
    }

    #[test]
    fn nombres_es_qu_en_evaluan_igual() {
        // Mismo resultado escribas SUM, SUMA o YAPAY.
        let esperado = SheetValue::Number(Decimal::from(60));
        assert_eq!(run("=SUM(A1:A3)"), esperado);
        assert_eq!(run("=SUMA(A1:A3)"), esperado);
        assert_eq!(run("=YAPAY(A1:A3)"), esperado); // quechua: añadir
    }

    #[test]
    fn logicas_y_texto_en_espanol() {
        assert_eq!(run(r#"=SI(A1>5, "alto", "bajo")"#), SheetValue::Text("alto".into()));
        assert_eq!(run(r#"=MAYUSC("hola")"#), SheetValue::Text("HOLA".into()));
        assert_eq!(run("=PROMEDIO(A1:A3)"), SheetValue::Number(Decimal::from(20)));
        assert_eq!(run("=CONTAR(A1:A3)"), SheetValue::Number(Decimal::from(3)));
    }

    #[test]
    fn funcion_inexistente_da_name_error() {
        assert_eq!(run("=NOEXISTE(A1)"), SheetValue::Error(SheetError::Name));
    }

    #[test]
    fn nombres_excel_es_con_punto_y_acento() {
        // El lexer ahora acepta punto (SUMAR.SI) y acentos (MÁXIMO, AÑO).
        assert_eq!(run("=MÁXIMO(A1:A3)"), SheetValue::Number(Decimal::from(30)));
        assert_eq!(run("=MÍNIMO(A1:A3)"), SheetValue::Number(Decimal::from(10)));
        // SUMAR.SI(rango, criterio, [rango_suma]) — suma A1:A3 donde >15.
        assert_eq!(
            run(r#"=SUMAR.SI(A1:A3, ">15")"#),
            SheetValue::Number(Decimal::from(50))
        );
        // El `.5` de un número sigue siendo número, no se pega al ident.
        assert_eq!(run("=A1*0.5"), SheetValue::Number(Decimal::from(5)));
    }
}
