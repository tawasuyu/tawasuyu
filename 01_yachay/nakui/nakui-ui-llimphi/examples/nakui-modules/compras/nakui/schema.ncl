# Schema autocontenido del módulo UI Compras: Cuenta (copia de accounting,
# porque el bundle del ejemplo no cruza paths) + Compra + Pago. La store
# del backend es compartida entre módulos UI, así que las Cuenta sembradas
# por `contabilidad` son las que el form de compras elige.
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

  Compra = {
    proveedor | String,
    neto | NoNegativo,
    impuesto | NoNegativo,
    total | NoNegativo,
    moneda | String,
    ..
  },

  Pago = {
    compra_id | String,
    monto | NoNegativo,
    moneda | String,
    ..
  },
}
