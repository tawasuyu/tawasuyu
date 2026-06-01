# Records ABIERTOS (`, ..`): los contracts de record de Nickel son cerrados
# por defecto y rebotaban el campo `id` (y `name`/`currency` en los seeds de
# Caja) que el kernel/los tests agregan. `estado` deja de ser requerido porque
# los seeds directos de Caja no lo traen (sólo lo pone `abrir_caja`); con el
# record abierto sigue conviviendo sin tipar. Campo requerido común = `saldo`.
{
  Caja = {
    saldo | Number,
    ..
  },
  Movimiento = {
    monto | Number,
    ..
  },
  Asiento = {
    detalle | String,
    ..
  },
}
