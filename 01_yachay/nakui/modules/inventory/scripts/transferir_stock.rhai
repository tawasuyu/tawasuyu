// transferir_stock — mueve `cantidad` unidades de un Stock a otro.
// Inputs ligados: `source` y `dest` (ambos entity Stock). Params: cantidad,
// timestamp, transfer_id.
//
// Conserva unidades: la regla `invariants.conserve` (Stock.cantidad agrupado
// por sku_id) la verifica el kernel a nivel de delta. El script sólo debe
// rechazar transferencias entre SKUs distintos; el sobregiro lo ataja el
// post-check no-negativo del schema.
if input.states.source.sku_id != input.states.dest.sku_id {
    throw "no se puede transferir entre SKUs distintos: "
        + input.states.source.sku_id + " != " + input.states.dest.sku_id;
}

[
    #{ op: "set",
       path: #{ entity: "Stock", id: input.ids.source, field: "cantidad" },
       value: input.states.source.cantidad - input.params.cantidad },
    #{ op: "set",
       path: #{ entity: "Stock", id: input.ids.dest, field: "cantidad" },
       value: input.states.dest.cantidad + input.params.cantidad },
    #{ op: "create", entity: "MovimientoStock", id: input.params.transfer_id,
       data: #{
           id: input.params.transfer_id,
           sku_id: input.states.source.sku_id,
           cantidad: input.params.cantidad,
           timestamp: input.params.timestamp,
       } },
]
