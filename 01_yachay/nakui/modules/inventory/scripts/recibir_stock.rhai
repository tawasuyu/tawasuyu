// recibir_stock — suma `cantidad` unidades a un Stock y deja constancia.
// Input ligado: `stock` (entity Stock). Params: cantidad, timestamp,
// movimiento_id.
[
    #{ op: "set",
       path: #{ entity: "Stock", id: input.ids.stock, field: "cantidad" },
       value: input.states.stock.cantidad + input.params.cantidad },
    #{ op: "create", entity: "MovimientoStock", id: input.params.movimiento_id,
       data: #{
           id: input.params.movimiento_id,
           sku_id: input.states.stock.sku_id,
           cantidad: input.params.cantidad,
           timestamp: input.params.timestamp,
       } },
]
