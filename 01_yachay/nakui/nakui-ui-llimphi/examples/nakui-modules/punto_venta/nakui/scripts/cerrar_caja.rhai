// Cierra la venta: marca su estado como cerrada.
[
    #{ op: "set",
       path: #{ entity: "Venta", id: input.ids.venta, field: "estado" },
       value: "cerrada" },
]
