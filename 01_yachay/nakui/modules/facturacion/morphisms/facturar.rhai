// facturar — emite una factura CON LÍNEAS y la ASIENTA en el libro. Como
// emitir_factura, pero el neto se calcula sumando las líneas. Inputs (de
// accounting): clientes_cta / ventas_cta / iva_cta. Params: cliente,
// fecha, factura_id, tasa (% IVA entero), lineas (array de
// { id, concepto, cantidad, precio_unitario }).
//
// Crea una `LineaFactura` por ítem (con su id de idempotencia que trae la
// fila), calcula neto = Σ cantidad×precio, impuesto = neto×tasa/100 y
// total = neto + impuesto, y asienta en deudor-normal:
//     debe  Clientes += total · haber Ventas -= neto · haber IVA -= impuesto
// Σ Δ saldo = total − neto − impuesto = 0 → la regla `conserve` lo exige.
import "fiscal" as f;

if input.states.clientes_cta.moneda != input.states.ventas_cta.moneda
   || input.states.ventas_cta.moneda != input.states.iva_cta.moneda {
    throw "las cuentas de la factura deben compartir moneda";
}
let moneda = input.states.clientes_cta.moneda;

// Datos fiscales del cliente OPCIONALES (ver emitir_factura): si vienen,
// se valida el dígito verificador y se guardan en la cabecera.
let fpais = input.params.pais;
let fdoc = input.params.documento;
let tiene_fiscal = (type_of(fdoc) == "string" && fdoc != "");
if tiene_fiscal && !f::dv_ok(fpais, fdoc) {
    throw "documento del cliente inválido (" + fpais + "): el dígito verificador no coincide (" + fdoc + ")";
}

let lineas = input.params.lineas;
if lineas.len() < 1 {
    throw "una factura necesita al menos una línea";
}

let ops = [];
let neto = 0;
for l in lineas {
    let subtotal = l.cantidad * l.precio_unitario;
    neto += subtotal;
    ops.push(#{ op: "create", entity: "LineaFactura", id: l.id,
        data: #{
            id: l.id,
            factura_id: input.params.factura_id,
            concepto: l.concepto,
            cantidad: l.cantidad,
            precio_unitario: l.precio_unitario,
            subtotal: subtotal,
        } });
}

let impuesto = neto * input.params.tasa / 100;
let total = neto + impuesto;

// Numeración legal correlativa: número = prefijo + el próximo de la
// secuencia, y se incrementa `proximo` en el mismo asiento. Atómico y sin
// huecos; determinista para el replay del log.
let numero = input.states.secuencia.prefijo + input.states.secuencia.proximo.to_string();
ops.push(#{ op: "set",
    path: #{ entity: "Secuencia", id: input.ids.secuencia, field: "proximo" },
    value: input.states.secuencia.proximo + 1 });

ops.push(#{ op: "set",
    path: #{ entity: "Cuenta", id: input.ids.clientes_cta, field: "saldo" },
    value: input.states.clientes_cta.saldo + total });
ops.push(#{ op: "set",
    path: #{ entity: "Cuenta", id: input.ids.ventas_cta, field: "saldo" },
    value: input.states.ventas_cta.saldo - neto });
ops.push(#{ op: "set",
    path: #{ entity: "Cuenta", id: input.ids.iva_cta, field: "saldo" },
    value: input.states.iva_cta.saldo - impuesto });

let factura = #{
    id: input.params.factura_id,
    numero: numero,
    cliente: input.params.cliente,
    fecha: input.params.fecha,
    estado: "emitida",
    neto: neto,
    impuesto: impuesto,
    total: total,
    moneda: moneda,
};
if tiene_fiscal {
    factura.pais = fpais;
    factura.documento = fdoc;
}
ops.push(#{ op: "create", entity: "Factura", id: input.params.factura_id, data: factura });

ops
