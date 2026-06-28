# Esquema del módulo Divisas (Nickel contracts) — ventas en moneda
# extranjera contabilizadas en la moneda FUNCIONAL (USD), con
# reconocimiento de la DIFERENCIA DE CAMBIO al cobrar.
#
# Cross-module: cruza el schema de accounting (Cuenta). El libro entero
# queda en USD funcional (las cuentas son USD), así que `conserve` cuadra
# como siempre; la moneda de origen sólo se guarda en la FacturaDivisa
# para referencia y para revaluar al cobro. La diferencia entre el
# equivalente cobrado (a la tasa del día) y el valor contabilizado (a la
# tasa de emisión) es la ganancia/pérdida de cambio — va a una cuenta de
# resultado, y el kernel garantiza que el asiento cierre.
#
# Tasas e importes en enteros: `tasa` es la cotización ×100 (1,10 = 110);
# monto_usd = monto_divisa × tasa / 100.
let NoNegativo = std.contract.from_predicate (fun n => std.is_number n && n >= 0) in
{
  # Una factura emitida en moneda extranjera. La crea `vender_divisa`.
  # Guarda el importe en divisa, su equivalente en USD a la tasa de
  # emisión, y esa tasa — para revaluar al cobro.
  FacturaDivisa = {
    cliente | String,
    moneda_origen | String,
    monto_divisa | NoNegativo,
    monto_usd | NoNegativo,
    tasa | NoNegativo,
    ..
  },

  # Constancia del cobro de una factura en divisa. `diferencia_cambio`
  # lleva signo: positivo = ganancia, negativo = pérdida.
  CobroDivisa = {
    factura_id | String,
    monto_usd | NoNegativo,
    diferencia_cambio | Number,
    ..
  },
}
