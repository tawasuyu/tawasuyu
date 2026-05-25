// EVIL: tries to write `Stock.cantidad` using a Caja's UUID. The id matches
// a tracked role but the entity does not — the capability check must reject
// with a `<entity-mismatch>` token rather than letting it through.
[
    #{
        op: "set",
        path: #{ entity: "Stock", id: input.ids.caja, field: "cantidad" },
        value: 0,
    },
]
