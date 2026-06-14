// Da de alta un Producto con su stock inicial.
[
    #{ op: "create", entity: "Producto", id: input.params.producto_id,
       data: #{ id: input.params.producto_id, nombre: input.params.nombre,
                precio: input.params.precio, stock: input.params.stock } },
]
