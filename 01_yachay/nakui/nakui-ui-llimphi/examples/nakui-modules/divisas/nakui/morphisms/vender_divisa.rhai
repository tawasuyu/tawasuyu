// vender_divisa — venta en moneda extranjera, contabilizada en la moneda
// FUNCIONAL (USD). Inputs: `clientes_cta` (CxC, USD) y `ventas_cta`
// (ingreso, USD). Params: cliente, moneda_origen, monto_divisa, tasa (×100),
// factura_id, fecha.
//
// Convierte: monto_usd = monto_divisa × tasa / 100, y asienta en USD:
//     debe  Clientes += monto_usd
//     haber Ventas   -= monto_usd
// Σ Δ saldo = 0 → conserve. Guarda en la FacturaDivisa el importe en
// divisa, su equivalente en USD y la tasa, para reconocer la diferencia de
// cambio cuando se cobre a otra cotización.
let monto_usd = input.params.monto_divisa * input.params.tasa / 100;

[
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.clientes_cta, field: "saldo" },
       value: input.states.clientes_cta.saldo + monto_usd },
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.ventas_cta, field: "saldo" },
       value: input.states.ventas_cta.saldo - monto_usd },
    #{ op: "create", entity: "FacturaDivisa", id: input.params.factura_id,
       data: #{
           id: input.params.factura_id,
           cliente: input.params.cliente,
           fecha: input.params.fecha,
           estado: "emitida",
           moneda_origen: input.params.moneda_origen,
           monto_divisa: input.params.monto_divisa,
           monto_usd: monto_usd,
           tasa: input.params.tasa,
       } },
]
