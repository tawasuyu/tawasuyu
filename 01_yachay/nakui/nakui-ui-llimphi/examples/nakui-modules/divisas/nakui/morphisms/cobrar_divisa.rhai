// cobrar_divisa — cobra una factura en divisa a la cotización del día y
// reconoce la DIFERENCIA DE CAMBIO. Inputs: `banco_cta` (USD),
// `clientes_cta` (USD), `resultado_cambio_cta` (cuenta de resultado por
// diferencias de cambio) y la `factura` (FacturaDivisa). Params: tasa
// (×100, cotización del cobro), cobro_id, fecha.
//
// monto_cobrado = factura.monto_divisa × tasa / 100  (equivalente USD hoy)
// carrying      = factura.monto_usd                  (valor contabilizado)
// diferencia    = monto_cobrado − carrying           (+ ganancia / − pérdida)
//
// Asienta en USD:
//     debe  Banco            += monto_cobrado
//     haber Clientes         -= carrying      (salda la CxC a su valor de libro)
//     ??    Resultado cambio -= diferencia    (un solo Set cubre ganancia y pérdida)
// Σ Δ = monto_cobrado − carrying − diferencia = 0 → conserve. El Set único
// sobre la cuenta de resultado funciona en ambos signos: si hay ganancia
// (diferencia>0) su saldo baja (ingreso); si hay pérdida (diferencia<0)
// sube (gasto/pérdida).
if input.states.factura.estado == "cobrada" {
    throw "la factura ya está cobrada";
}

let monto_cobrado = input.states.factura.monto_divisa * input.params.tasa / 100;
let carrying = input.states.factura.monto_usd;
let diferencia = monto_cobrado - carrying;

[
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.banco_cta, field: "saldo" },
       value: input.states.banco_cta.saldo + monto_cobrado },
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.clientes_cta, field: "saldo" },
       value: input.states.clientes_cta.saldo - carrying },
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.resultado_cambio_cta, field: "saldo" },
       value: input.states.resultado_cambio_cta.saldo - diferencia },
    #{ op: "set",
       path: #{ entity: "FacturaDivisa", id: input.ids.factura, field: "estado" },
       value: "cobrada" },
    #{ op: "create", entity: "CobroDivisa", id: input.params.cobro_id,
       data: #{
           id: input.params.cobro_id,
           factura_id: input.ids.factura,
           fecha: input.params.fecha,
           monto_usd: monto_cobrado,
           diferencia_cambio: diferencia,
       } },
]
