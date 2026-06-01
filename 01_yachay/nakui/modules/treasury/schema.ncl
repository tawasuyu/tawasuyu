# Records ABIERTOS (`, ..`): los contracts de record de Nickel son cerrados
# por defecto y rebotaban el campo `id` (y `name`/`currency` en los seeds de
# Caja) que el kernel/los tests agregan. `estado` deja de ser requerido porque
# los seeds directos de Caja no lo traen (sólo lo pone `abrir_caja`); con el
# record abierto sigue conviviendo sin tipar. Campo requerido común = `saldo`.
#
# `saldo` y `monto` no-negativos: el post-check KCL del kernel rebota cualquier
# op que deje una Caja en negativo (sobregiro de transfer_between_cajas / cash
# move "out") y cualquier Movimiento con monto negativo (test bad_created_record).
let NoNegativo = std.contract.from_predicate (fun n => std.is_number n && n >= 0) in
{
  Caja = {
    saldo | NoNegativo,
    ..
  },
  Movimiento = {
    monto | NoNegativo,
    ..
  },
  # Constancia append-only de una transferencia entre cajas.
  Transferencia = {
    monto | NoNegativo,
    ..
  },
}
