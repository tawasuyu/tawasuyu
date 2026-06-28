// recibir_mercaderia — recibe `cantidad` unidades a `costo_unitario` en el
// stock de un Producto y lo ASIENTA. Inputs: `producto` (Producto),
// `inventario_cta` (activo) y `contrapartida_cta` (Proveedores en compra a
// crédito, Banco al contado). Params: cantidad, costo_unitario, fecha,
// movimiento_id.
//
// Sube cantidad y valor_total juntos: así el costo promedio ponderado
// (valor_total / cantidad) queda correcto sin guardarlo aparte. Asienta en
// deudor-normal:
//     debe  Inventario     += costo_total   (sube el activo)
//     haber Contrapartida  -= costo_total   (deuda con proveedor / sale del banco)
// Σ Δ Cuenta.saldo = 0 → la regla `conserve` lo exige. La cuenta de
// Inventario y el valor_total del Producto se mueven por el mismo importe,
// manteniéndolos en sincronía.
let cantidad = input.params.cantidad;
let costo_total = cantidad * input.params.costo_unitario;

[
    #{ op: "set",
       path: #{ entity: "Producto", id: input.ids.producto, field: "cantidad" },
       value: input.states.producto.cantidad + cantidad },
    #{ op: "set",
       path: #{ entity: "Producto", id: input.ids.producto, field: "valor_total" },
       value: input.states.producto.valor_total + costo_total },
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.inventario_cta, field: "saldo" },
       value: input.states.inventario_cta.saldo + costo_total },
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.contrapartida_cta, field: "saldo" },
       value: input.states.contrapartida_cta.saldo - costo_total },
    #{ op: "create", entity: "MovimientoStock", id: input.params.movimiento_id,
       data: #{
           id: input.params.movimiento_id,
           producto_id: input.ids.producto,
           tipo: "entrada",
           cantidad: cantidad,
           costo_total: costo_total,
           fecha: input.params.fecha,
       } },
]
