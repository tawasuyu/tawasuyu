// facturar_cotizacion — segundo tramo del puente CRM → facturación: toma una
// Cotización en "borrador" y emite la Factura de venta que ASIENTA en el libro
// de accounting, dejando la cotización en "facturada".
//
// Inputs: `cotizacion` (Cotizacion del CRM) + tres cuentas de accounting:
//   `clientes_cta` (CxC, activo), `ventas_cta` (ingreso), `iva_cta` (IVA por
//   pagar, pasivo). Params: factura_id, fecha, tasa (% IVA entero).
//
// El neto sale del `monto` de la cotización (no se re-teclea). Asiento en
// convención deudor-normal, idéntico a facturacion::emitir_factura:
//     debe  Clientes += total
//     haber Ventas   -= neto
//     haber IVA      -= impuesto
// Σ Δ saldo = total − neto − impuesto = 0 → la regla `conserve` del kernel
// (Cuenta.saldo group_by moneda) garantiza que la factura no descuadra el
// libro. Reusa el contrato Factura de facturacion (bundleado por nsmc.json).

// 1) La cotización debe estar en borrador: no se factura dos veces.
let estado = input.states.cotizacion.estado;
if estado != "borrador" {
    throw "la cotización no está en borrador (estado: " + estado + "): no se puede facturar";
}

// 2) Las tres cuentas comparten moneda, y debe coincidir con la cotización.
if input.states.clientes_cta.moneda != input.states.ventas_cta.moneda
   || input.states.ventas_cta.moneda != input.states.iva_cta.moneda {
    throw "las cuentas de la factura deben compartir moneda";
}
let moneda = input.states.clientes_cta.moneda;
if input.states.cotizacion.currency != moneda {
    throw "la moneda de la cotización (" + input.states.cotizacion.currency
        + ") no coincide con la de las cuentas (" + moneda + ")";
}

let neto = input.states.cotizacion.monto;
let impuesto = neto * input.params.tasa / 100;
let total = neto + impuesto;

let factura = #{
    id: input.params.factura_id,
    cliente: input.states.cotizacion.cliente_id,
    fecha: input.params.fecha,
    estado: "emitida",
    neto: neto,
    impuesto: impuesto,
    total: total,
    moneda: moneda,
    cotizacion_id: input.ids.cotizacion,
};

[
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.clientes_cta, field: "saldo" },
       value: input.states.clientes_cta.saldo + total },
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.ventas_cta, field: "saldo" },
       value: input.states.ventas_cta.saldo - neto },
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.iva_cta, field: "saldo" },
       value: input.states.iva_cta.saldo - impuesto },
    #{ op: "create", entity: "Factura", id: input.params.factura_id, data: factura },
    #{ op: "set",
       path: #{ entity: "Cotizacion", id: input.ids.cotizacion, field: "estado" },
       value: "facturada" },
]
