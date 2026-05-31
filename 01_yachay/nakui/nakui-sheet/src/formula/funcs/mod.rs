//! Funciones builtin. El dispatch va por nombre UPPERCASE (el parser
//! ya normaliza). Si el nombre no existe devolvemos `#NAME?` —
//! coherente con Excel cuando teclea uno mal el nombre de una
//! función.
//!
//! Cada función ignora celdas vacías al agregar (igual que SUM en
//! Excel), pero `COUNT` solo cuenta los numéricos. Texto que no parsea
//! a número produce `#VALUE!` solo en contextos numéricos puros (es
//! decir: las agregadas lo saltan, mientras que `1 + "abc"` sí cae).

use super::ast::FormulaArg;
use crate::value::{SheetError, SheetValue};
use rust_decimal::Decimal;

pub fn dispatch(name: &str, args: &[FormulaArg]) -> SheetValue {
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

// --- Submódulos por categoría de función. dispatch() rutea a todos; los
// helpers compartidos (arity, scalar_*, flatten_numbers...) se re-exportan
// pub(crate) desde aquí para que cada módulo los vea vía `use super::*`. ---
mod aggregate;
mod criteria;
mod datetime;
mod lookup;
mod scalar;
#[cfg(test)]
mod tests;

pub(crate) use aggregate::*;
pub(crate) use criteria::*;
pub(crate) use datetime::*;
pub(crate) use lookup::*;
pub(crate) use scalar::*;
