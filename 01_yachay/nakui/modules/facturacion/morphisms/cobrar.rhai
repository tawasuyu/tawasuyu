// cobrar — cobra una factura: mueve su total de Clientes (CxC) a Banco y
// la marca como cobrada. Cierra el ciclo de cuentas por cobrar. Inputs:
// `banco_cta` (activo), `clientes_cta` (CxC, activo) y la `factura`
// (entity Factura). Params: fecha, cobro_id.
//
// Asienta en deudor-normal:
//     debe  Banco    += total   (entra plata al banco)
//     haber Clientes -= total   (baja la cuenta por cobrar)
// Σ Δ saldo = total − total = 0 → la regla `conserve` lo exige. Además
// marca `factura.estado = "cobrada"` (Set sobre la Factura, otra entidad:
// no afecta la conservación de Cuenta) y deja un `Cobro` de constancia.
if input.states.banco_cta.moneda != input.states.clientes_cta.moneda
   || input.states.clientes_cta.moneda != input.states.factura.moneda {
    throw "banco, clientes y factura deben compartir moneda";
}
if input.states.factura.estado == "cobrada" {
    throw "la factura ya está cobrada";
}

let monto = input.states.factura.total;

[
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.banco_cta, field: "saldo" },
       value: input.states.banco_cta.saldo + monto },
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.clientes_cta, field: "saldo" },
       value: input.states.clientes_cta.saldo - monto },
    #{ op: "set",
       path: #{ entity: "Factura", id: input.ids.factura, field: "estado" },
       value: "cobrada" },
    #{ op: "create", entity: "Cobro", id: input.params.cobro_id,
       data: #{
           id: input.params.cobro_id,
           factura_id: input.ids.factura,
           monto: monto,
           fecha: input.params.fecha,
           moneda: input.states.factura.moneda,
       } },
]
