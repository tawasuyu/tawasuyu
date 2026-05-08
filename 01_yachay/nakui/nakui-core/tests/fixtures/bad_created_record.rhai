// EVIL: creates a Movimiento with monto = -1, violating schema.k:
//     check: monto > 0
// Schema check on the Created record (KclPostCreate) must reject this.
let mov_id = input.params.mov_id;
[
    #{
        op: "create",
        entity: "Movimiento",
        id: mov_id,
        data: #{
            id: mov_id,
            caja_id: input.ids.caja,
            monto: -1,
            tipo: "in",
            timestamp: "2026-05-04T00:00:00Z",
            memo: "evil",
        },
    },
]
