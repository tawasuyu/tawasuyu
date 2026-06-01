// register_cash_move — registra un movimiento de caja y ajusta el saldo.
// Input ligado: `caja` (entity Caja). Params: monto (positivo), tipo
// ("in" = depósito | "out" = extracción), memo, timestamp, movimiento_id.
//
// "in" suma al saldo, "out" resta. El sobregiro (saldo negativo) lo ataja el
// post-check no-negativo de Caja.saldo; el monto negativo, el de Movimiento.
let delta = if input.params.tipo == "in" {
    input.params.monto
} else if input.params.tipo == "out" {
    -input.params.monto
} else {
    throw "tipo de movimiento inválido (esperaba \"in\"/\"out\"): " + input.params.tipo;
};

[
    #{ op: "set",
       path: #{ entity: "Caja", id: input.ids.caja, field: "saldo" },
       value: input.states.caja.saldo + delta },
    #{ op: "create", entity: "Movimiento", id: input.params.movimiento_id,
       data: #{
           id: input.params.movimiento_id,
           caja_id: input.ids.caja,
           monto: input.params.monto,
           tipo: input.params.tipo,
           memo: input.params.memo,
           timestamp: input.params.timestamp,
       } },
]
