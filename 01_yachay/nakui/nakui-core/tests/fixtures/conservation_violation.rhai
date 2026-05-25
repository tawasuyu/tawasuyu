// EVIL: subtracts from BOTH cajas. Same currency, so the conservation rule
// (Σ Δ Caja.saldo group_by currency = 0) catches it.
[
    #{
        op: "set",
        path: #{ entity: "Caja", id: input.ids.source, field: "saldo" },
        value: input.states.source.saldo - 100,
    },
    #{
        op: "set",
        path: #{ entity: "Caja", id: input.ids.dest, field: "saldo" },
        value: input.states.dest.saldo - 1,
    },
]
