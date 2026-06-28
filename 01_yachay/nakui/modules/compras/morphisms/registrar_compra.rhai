// registrar_compra — registra una factura de compra (vendor bill) y la
// ASIENTA en el libro. Inputs (cuentas de accounting): `compra_cta` (la
// cuenta que recibe el neto: un gasto o un activo de inventario),
// `iva_cta` (IVA crédito fiscal, activo) y `proveedores_cta` (pasivo).
// Params: proveedor, fecha, compra_id, neto, tasa (% IVA entero).
//
// Calcula impuesto = neto×tasa/100 y total = neto + impuesto. Asienta en
// convención deudor-normal:
//     debe  Gasto/Inventario += neto      (entra el costo)
//     debe  IVA crédito fisc. += impuesto (crédito contra el fisco)
//     haber Proveedores       -= total    (sube la deuda con el proveedor)
// Σ Δ saldo = neto + impuesto − total = 0 → la regla `conserve` lo exige:
// una compra nunca descuadra el libro.
// Datos fiscales del proveedor OPCIONALES: si los params traen `documento`
// (y `pais`), se valida el dígito verificador acá (módulo Rhai "fiscal") y
// se guardan en la compra; el formato lo gatea el contrato Nickel. Sin
// `documento`, la compra se registra igual (retrocompat).
import "fiscal" as f;

if input.states.compra_cta.moneda != input.states.iva_cta.moneda
   || input.states.iva_cta.moneda != input.states.proveedores_cta.moneda {
    throw "las cuentas de la compra deben compartir moneda";
}
let moneda = input.states.compra_cta.moneda;

let neto = input.params.neto;
let impuesto = neto * input.params.tasa / 100;
let total = neto + impuesto;

let fpais = input.params.pais;
let fdoc = input.params.documento;
let tiene_fiscal = (type_of(fdoc) == "string" && fdoc != "");
if tiene_fiscal && !f::dv_ok(fpais, fdoc) {
    throw "documento del proveedor inválido (" + fpais + "): el dígito verificador no coincide (" + fdoc + ")";
}

let compra = #{
    id: input.params.compra_id,
    proveedor: input.params.proveedor,
    fecha: input.params.fecha,
    estado: "registrada",
    neto: neto,
    impuesto: impuesto,
    total: total,
    moneda: moneda,
};
if tiene_fiscal {
    compra.pais = fpais;
    compra.documento = fdoc;
}

[
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.compra_cta, field: "saldo" },
       value: input.states.compra_cta.saldo + neto },
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.iva_cta, field: "saldo" },
       value: input.states.iva_cta.saldo + impuesto },
    #{ op: "set",
       path: #{ entity: "Cuenta", id: input.ids.proveedores_cta, field: "saldo" },
       value: input.states.proveedores_cta.saldo - total },
    #{ op: "create", entity: "Compra", id: input.params.compra_id, data: compra },
]
