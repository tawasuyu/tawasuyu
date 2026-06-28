# Esquema del módulo Facturacion (Nickel contracts) — emisión de facturas
# de venta que ASIENTAN en el libro de `accounting`.
#
# Facturacion es CROSS-MODULE: su `nsmc.json` arma el bundle con el schema
# de accounting (Cuenta) y agrega sólo la entidad Factura. El morfismo
# `emitir_factura` liga tres cuentas (Clientes/Ventas/IVA) y mueve sus
# saldos de forma balanceada — el kernel exige Σ Δ saldo = 0 vía la regla
# `conserve` sobre Cuenta.saldo (la misma partida doble de accounting).
#
# Importes en unidades enteras de moneda (sin centavos). `impuesto` se
# deriva de `neto` y la tasa; `total = neto + impuesto` por construcción.
let NoNegativo = std.contract.from_predicate (fun n => std.is_number n && n >= 0) in
{
  # Una factura de venta emitida. La crea `emitir_factura`. Cabecera con
  # los importes ya calculados; el detalle de líneas queda para una
  # iteración futura (necesita líneas planas tipo Renglon).
  Factura = {
    cliente | String,
    neto | NoNegativo,
    impuesto | NoNegativo,
    total | NoNegativo,
    moneda | String,
    ..
  },

  # Una línea (renglón) de una factura. La crea `facturar`, una por ítem.
  # Plana (no array anidado, por el bug de Nickel 0.18); `factura_id` la
  # liga a su cabecera. `subtotal = cantidad × precio_unitario`.
  LineaFactura = {
    factura_id | String,
    concepto | String,
    cantidad | NoNegativo,
    precio_unitario | NoNegativo,
    subtotal | NoNegativo,
    ..
  },

  # Constancia append-only del cobro de una factura. La crea `cobrar`:
  # mueve el total de Clientes (CxC) a Banco y marca la factura como
  # cobrada. `monto` = total cobrado; `factura_id` la liga a su factura.
  Cobro = {
    factura_id | String,
    monto | NoNegativo,
    moneda | String,
    ..
  },
}
