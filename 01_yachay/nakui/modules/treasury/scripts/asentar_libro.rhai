// Asienta el saldo actual de la caja en el libro (Asiento).
[
    #{ op: "create", entity: "Asiento", id: input.params.asiento_id,
       data: #{ id: input.params.asiento_id,
                detalle: "saldo=" + input.states.caja.saldo } },
]
