// transfer_between_cajas — mueve `monto` de la caja source a la dest.
// Inputs ligados: `source` y `dest` (ambos entity Caja). Params: monto,
// memo, timestamp, transfer_id.
//
// Conserva el total por moneda: la regla `invariants.conserve` (Caja.saldo
// agrupado por currency) la verifica el kernel a nivel de delta. El script
// sólo rechaza transferencias entre monedas distintas; el sobregiro lo ataja
// el post-check no-negativo de Caja.saldo.
if input.states.source.currency != input.states.dest.currency {
    throw "no se puede transferir entre monedas distintas: "
        + input.states.source.currency + " != " + input.states.dest.currency;
}

[
    #{ op: "set",
       path: #{ entity: "Caja", id: input.ids.source, field: "saldo" },
       value: input.states.source.saldo - input.params.monto },
    #{ op: "set",
       path: #{ entity: "Caja", id: input.ids.dest, field: "saldo" },
       value: input.states.dest.saldo + input.params.monto },
    #{ op: "create", entity: "Transferencia", id: input.params.transfer_id,
       data: #{
           id: input.params.transfer_id,
           source_id: input.ids.source,
           dest_id: input.ids.dest,
           monto: input.params.monto,
           memo: input.params.memo,
           timestamp: input.params.timestamp,
       } },
]
