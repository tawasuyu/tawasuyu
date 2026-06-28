# Esquema del módulo Accounting (Nickel contracts) — contabilidad de
# PARTIDA DOBLE sobre el motor de conservación de nakui.
#
# Convención DEUDOR-NORMAL: `Cuenta.saldo` vive en unidades mínimas
# (centavos, entero) y mide débitos menos créditos. El débito SUMA al
# saldo, el crédito lo RESTA. Bajo esta convención un asiento balanceado
# cumple Σ Δ saldo = 0 — y la invariante `conserve` del kernel (ver
# `nsmc.json`) ES la partida doble: no la garantiza el código de app, la
# rebota el executor a nivel de delta.
#
# OJO, diferencia dura con `treasury.Caja`: `Cuenta.saldo` NO es
# no-negativo. Una cuenta de pasivo / patrimonio / ingreso, leída en
# convención deudor-normal, tiene saldo negativo de forma legítima (su
# naturaleza es acreedora). Forzar no-negatividad acá rompería la
# contabilidad. Lo único que se exige positivo es el `monto` del Asiento.
let NoNegativo = std.contract.from_predicate (fun n => std.is_number n && n >= 0) in
{
  # Una cuenta del plan contable. Se siembra directamente (como Caja),
  # no hay morfismo de apertura. `tipo` ∈
  # {activo, pasivo, patrimonio, ingreso, gasto}; el reporte interpreta
  # el signo deudor-normal del saldo según el tipo.
  Cuenta = {
    codigo | String,
    nombre | String,
    tipo | String,
    saldo | Number,
    moneda | String,
    ..
  },

  # Constancia append-only de un asiento de dos patas (un débito contra
  # un crédito por el mismo `monto`). La crea `asentar`. El kernel
  # garantiza débito == crédito vía la regla `conserve`; el monto
  # negativo lo ataja el post-check NoNegativo de acá.
  Asiento = {
    monto | NoNegativo,
    debito_id | String,
    credito_id | String,
    moneda | String,
    ..
  },
}
