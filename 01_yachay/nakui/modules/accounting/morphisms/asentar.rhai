// asentar — registra un asiento de partida doble: DEBITA la cuenta
// `debito` y ACREDITA la cuenta `credito` por el mismo `monto`.
// Inputs ligados: `debito` y `credito` (ambos entity Cuenta). Params:
// monto (positivo, centavos), glosa, fecha, diario, asiento_id.
//
// Convención deudor-normal (ver schema.ncl): el débito SUMA al saldo, el
// crédito RESTA. Por construcción Σ Δ saldo = +monto - monto = 0, así que
// la invariante `conserve` (Cuenta.saldo agrupado por moneda) la verifica
// el kernel: ESA es la partida doble. El monto negativo lo rebota el
// post-check NoNegativo de Asiento.monto; la mezcla de monedas, el throw
// de acá (y, de forma redundante, el conserve por moneda, que dejaría
// buckets no-cero).
//
// Nota: el executor rechaza ligar la misma cuenta a `debito` y `credito`
// (DuplicateInputId) — un asiento contra una sola cuenta no tiene sentido.
if input.states.debito.moneda != input.states.credito.moneda {
    throw "no se puede asentar entre monedas distintas: "
        + input.states.debito.moneda + " != " + input.states.credito.moneda;
}

[
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.debito, field: "saldo" },
       value: input.states.debito.saldo + input.params.monto },
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.credito, field: "saldo" },
       value: input.states.credito.saldo - input.params.monto },
    #{ op: "create", entity: "Asiento", id: input.params.asiento_id,
       data: #{
           id: input.params.asiento_id,
           fecha: input.params.fecha,
           glosa: input.params.glosa,
           diario: input.params.diario,
           debito_id: input.ids.debito,
           credito_id: input.ids.credito,
           monto: input.params.monto,
           moneda: input.states.debito.moneda,
       } },
]
