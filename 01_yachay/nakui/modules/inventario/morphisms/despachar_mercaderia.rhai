// despachar_mercaderia — saca `cantidad` unidades del stock a COSTO
// PROMEDIO PONDERADO y asienta el costo de mercadería vendida (COGS).
// Inputs: `producto` (Producto), `inventario_cta` (activo) y `cogs_cta`
// (gasto). Params: cantidad, fecha, movimiento_id.
//
// El costo de salida es proporcional al valor en mano:
//     costo_salida = valor_total × cantidad / cantidad_actual
// (entero; EXACTO cuando se despacha todo → valor_total queda en 0). Asienta:
//     debe  COGS       += costo_salida   (gasto del costo vendido)
//     haber Inventario -= costo_salida   (baja el activo)
// Σ Δ Cuenta.saldo = 0 → `conserve`. El sobregiro (cantidad > stock) lo
// rebota el post-check NoNegativo de Producto.cantidad / valor_total.
let cantidad = input.params.cantidad;
let actual = input.states.producto.cantidad;
if actual <= 0 {
    throw "el producto no tiene stock";
}
let costo_salida = input.states.producto.valor_total * cantidad / actual;

[
    #{ op: "set",
       path: #{ entity: "Producto", id: input.ids.producto, field: "cantidad" },
       value: input.states.producto.cantidad - cantidad },
    #{ op: "set",
       path: #{ entity: "Producto", id: input.ids.producto, field: "valor_total" },
       value: input.states.producto.valor_total - costo_salida },
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.cogs_cta, field: "saldo" },
       value: input.states.cogs_cta.saldo + costo_salida },
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.inventario_cta, field: "saldo" },
       value: input.states.inventario_cta.saldo - costo_salida },
    #{ op: "create", entity: "MovimientoStock", id: input.params.movimiento_id,
       data: #{
           id: input.params.movimiento_id,
           producto_id: input.ids.producto,
           tipo: "salida",
           cantidad: cantidad,
           costo_total: costo_salida,
           fecha: input.params.fecha,
       } },
]
