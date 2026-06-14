// Repone stock del producto (suma unidades al saldo actual).
[
    #{ op: "set",
       path: #{ entity: "Producto", id: input.ids.producto, field: "stock" },
       value: input.states.producto.stock + input.params.cantidad },
]
