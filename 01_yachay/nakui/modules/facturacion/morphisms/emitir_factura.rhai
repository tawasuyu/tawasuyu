// emitir_factura — emite una factura de venta y la ASIENTA en el libro.
// Inputs (cuentas de accounting): `clientes_cta` (cuenta por cobrar,
// activo), `ventas_cta` (ingreso), `iva_cta` (IVA por pagar, pasivo).
// Params: cliente, fecha, factura_id, neto (base imponible), tasa (% IVA
// entero).
//
// Calcula impuesto = neto × tasa / 100 (división entera, trunca) y
// total = neto + impuesto. Asienta en convención deudor-normal:
//     debe  Clientes += total
//     haber Ventas   -= neto
//     haber IVA      -= impuesto
// Σ Δ saldo = total − neto − impuesto = 0 por construcción → la regla
// `conserve` (Cuenta.saldo group_by moneda) la verifica el kernel: la
// factura no puede descuadrar el libro. El executor rechaza ligar la
// misma cuenta a dos roles (DuplicateInputId) y deja negativos sólo donde
// la cuenta lo admite (ingreso/pasivo acreedor).
//
// Datos fiscales del cliente OPCIONALES: si los params traen `documento`
// (y `pais`), se valida el dígito verificador acá (módulo Rhai "fiscal") y
// se guardan en la factura; el formato lo gatea el contrato Nickel. Sin
// `documento`, la factura se emite igual (retrocompat).
import "fiscal" as f;

if input.states.clientes_cta.moneda != input.states.ventas_cta.moneda
   || input.states.ventas_cta.moneda != input.states.iva_cta.moneda {
    throw "las cuentas de la factura deben compartir moneda";
}
let moneda = input.states.clientes_cta.moneda;

let neto = input.params.neto;
let impuesto = neto * input.params.tasa / 100;
let total = neto + impuesto;

let fpais = input.params.pais;
let fdoc_raw = input.params.documento;
let tiene_fiscal = (type_of(fdoc_raw) == "string" && fdoc_raw != "");
let fdoc = "";
if tiene_fiscal {
    fdoc = f::normalizar(fpais, fdoc_raw); // forma canónica del país
    if !f::dv_ok(fpais, fdoc) {
        throw "documento del cliente inválido (" + fpais + "): el dígito verificador no coincide (" + fdoc_raw + ")";
    }
}

let factura = #{
    id: input.params.factura_id,
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
]
