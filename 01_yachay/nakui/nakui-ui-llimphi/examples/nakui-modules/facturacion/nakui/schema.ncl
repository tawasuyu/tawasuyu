# Schema autocontenido del módulo UI Facturacion: incluye Cuenta (copia
# de accounting, porque el bundle del ejemplo no cruza paths) + Factura.
# La store del backend es compartida entre módulos UI, así que las Cuenta
# sembradas por `contabilidad` son las que el form de facturación elige.
let NoNegativo = std.contract.from_predicate (fun n => std.is_number n && n >= 0) in
{
  Cuenta = {
    codigo | String,
    nombre | String,
    tipo | String,
    saldo | Number,
    moneda | String,
    ..
  },

  Factura = {
    cliente | String,
    neto | NoNegativo,
    impuesto | NoNegativo,
    total | NoNegativo,
    moneda | String,
    ..
  },

  LineaFactura = {
    factura_id | String,
    concepto | String,
    cantidad | NoNegativo,
    precio_unitario | NoNegativo,
    subtotal | NoNegativo,
    ..
  },

  Cobro = {
    factura_id | String,
    monto | NoNegativo,
    moneda | String,
    ..
  },

  Secuencia = {
    codigo | String,
    prefijo | String,
    proximo | NoNegativo,
    ..
  },
}
