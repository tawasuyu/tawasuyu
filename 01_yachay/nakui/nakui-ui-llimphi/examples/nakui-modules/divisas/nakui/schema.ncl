# Schema autocontenido del módulo UI Divisas: Cuenta (copia de accounting)
# + FacturaDivisa + CobroDivisa. Comparte la store con contabilidad.
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

  FacturaDivisa = {
    cliente | String,
    moneda_origen | String,
    monto_divisa | NoNegativo,
    monto_usd | NoNegativo,
    tasa | NoNegativo,
    ..
  },

  CobroDivisa = {
    factura_id | String,
    monto_usd | NoNegativo,
    diferencia_cambio | Number,
    ..
  },
}
