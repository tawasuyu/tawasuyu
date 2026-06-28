# Esquema del módulo Inventario (Nickel contracts) — stock valuado a
# COSTO PROMEDIO PONDERADO, con reflejo contable obligatorio en `accounting`
# (la "partida doble extendida a recursos físicos"). Cross-module: el
# `nsmc.json` cruza el schema de accounting (Cuenta) y agrega Producto +
# MovimientoStock.
#
# Cada Producto lleva `cantidad` (unidades en mano) y `valor_total` (valor
# de ese stock en unidades enteras de moneda). El costo promedio es
# `valor_total / cantidad`. `recibir_mercaderia` sube ambos juntos
# (mantiene el promedio); `despachar_mercaderia` baja el valor proporcional
# al costo promedio. La cuenta contable de Inventario se mueve por el mismo
# importe en cada operación, así que `saldo` ≈ Σ `valor_total` por
# construcción. El kernel garantiza que el ASIENTO cuadre (Σ Δ Cuenta.saldo
# = 0 vía `conserve`); el no-negativo de `cantidad`/`valor_total` ataja el
# sobregiro de stock.
let NoNegativo = std.contract.from_predicate (fun n => std.is_number n && n >= 0) in
{
  # Un ítem de inventario. Se siembra con cantidad/valor 0; el stock entra
  # por `recibir_mercaderia`.
  Producto = {
    sku | String,
    nombre | String,
    cantidad | NoNegativo,
    valor_total | NoNegativo,
    moneda | String,
    ..
  },

  # Registro append-only de un movimiento de stock (entrada/salida) con su
  # costo. La crean `recibir_mercaderia` / `despachar_mercaderia`.
  MovimientoStock = {
    producto_id | String,
    tipo | String,
    cantidad | NoNegativo,
    costo_total | NoNegativo,
    ..
  },
}
