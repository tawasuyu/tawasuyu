// Crea una Caja nueva con saldo inicial 0.
[
    #{ op: "create", entity: "Caja", id: input.params.caja_id,
       data: #{ id: input.params.caja_id, saldo: 0, estado: "abierta" } },
]
