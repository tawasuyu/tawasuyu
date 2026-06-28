// pagar — paga una compra: mueve su total de Proveedores a Banco y la
// marca como pagada. Cierra el ciclo de cuentas por pagar (espejo de
// `cobrar`). Inputs: `banco_cta` (activo), `proveedores_cta` (pasivo) y la
// `compra` (entity Compra). Params: fecha, pago_id.
//
// Asienta en deudor-normal:
//     debe  Proveedores += total   (baja la deuda con el proveedor)
//     haber Banco        -= total   (sale plata del banco)
// Σ Δ saldo = total − total = 0 → la regla `conserve` lo exige. Marca
// `compra.estado = "pagada"` (Set sobre la Compra, otra entidad: fuera de
// la conservación de Cuenta) y deja un `Pago` de constancia.
if input.states.banco_cta.moneda != input.states.proveedores_cta.moneda
   || input.states.proveedores_cta.moneda != input.states.compra.moneda {
    throw "banco, proveedores y compra deben compartir moneda";
}
if input.states.compra.estado == "pagada" {
    throw "la compra ya está pagada";
}

let monto = input.states.compra.total;

[
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.proveedores_cta, field: "saldo" },
       value: input.states.proveedores_cta.saldo + monto },
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.banco_cta, field: "saldo" },
       value: input.states.banco_cta.saldo - monto },
    #{ op: "set",
       path: #{ entity: "Compra", id: input.ids.compra, field: "estado" },
       value: "pagada" },
    #{ op: "create", entity: "Pago", id: input.params.pago_id,
       data: #{
           id: input.params.pago_id,
           compra_id: input.ids.compra,
           monto: monto,
           fecha: input.params.fecha,
           moneda: input.states.compra.moneda,
       } },
]
