// Aplica el monto del Movimiento al saldo de la caja.
[
    #{ op: "set",
       path: #{ entity: "Caja", id: input.ids.caja, field: "saldo" },
       value: input.states.caja.saldo + input.params.monto },
]
