// vender — venta de `cantidad` unidades de un Stock cobrada a una Caja.
// Inputs ligados: `stock` (entity Stock) y `caja` (entity Caja, de treasury).
// Params: cantidad, precio_unitario, timestamp, venta_id.
//
// No es conservativa (baja stock, sube caja): por eso `nsmc.json` NO declara
// `invariants.conserve` — el kernel la deja pasar limpia. El sobregiro de
// stock lo ataja el post-check no-negativo de Stock.cantidad (inventory).
let total = input.params.cantidad * input.params.precio_unitario;

[
    #{ op: "set",
       path: #{ entity: "Stock", id: input.ids.stock, field: "cantidad" },
       value: input.states.stock.cantidad - input.params.cantidad },
    #{ op: "set",
       path: #{ entity: "Caja", id: input.ids.caja, field: "saldo" },
       value: input.states.caja.saldo + total },
    #{ op: "create", entity: "Venta", id: input.params.venta_id,
       data: #{
           id: input.params.venta_id,
           cantidad: input.params.cantidad,
           total: total,
           currency: input.states.caja.currency,
       } },
]
