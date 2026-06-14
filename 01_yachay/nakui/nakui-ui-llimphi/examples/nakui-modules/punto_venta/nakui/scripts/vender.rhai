// Descuenta el stock del producto y asienta una línea de venta con su importe.
[
    #{ op: "set",
       path: #{ entity: "Producto", id: input.ids.producto, field: "stock" },
       value: input.states.producto.stock - input.params.cantidad },
    #{ op: "create", entity: "LineaVenta", id: input.params.linea_id,
       data: #{ id: input.params.linea_id, producto: input.ids.producto,
                cantidad: input.params.cantidad,
                importe: input.params.precio * input.params.cantidad } },
]
