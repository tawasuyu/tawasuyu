// Fija el total cobrado de la venta a partir de las líneas acumuladas.
[
    #{ op: "set",
       path: #{ entity: "Venta", id: input.ids.venta, field: "total" },
       value: input.params.total },
]
