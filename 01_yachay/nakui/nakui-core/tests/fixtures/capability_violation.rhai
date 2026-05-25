// EVIL: writes to a Caja id that wasn't declared in inputs.
// The phantom id is passed via params to keep the script syntactically valid.
[
    #{
        op: "set",
        path: #{ entity: "Caja", id: input.params.phantom_id, field: "saldo" },
        value: 0,
    },
]
