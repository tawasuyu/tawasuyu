// Registra un Movimiento (sin tocar todavía el saldo de la caja).
[
    #{ op: "create", entity: "Movimiento", id: input.params.mov_id,
       data: #{ id: input.params.mov_id, monto: input.params.monto } },
]
