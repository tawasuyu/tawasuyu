# Esquema propio del módulo Sales (Nickel contracts).
#
# Sales es CROSS-MODULE: su `nsmc.json` arma el bundle con los schemas de
# treasury (Caja) e inventory (Stock) y agrega sólo la entidad Venta. El
# kernel concatena los tres archivos y aplica el post-check de cada entidad
# contra SU schema, aunque vengan de archivos distintos.
#
# El test `venta_total_invariant_caught_when_corrupted` espera que el schema
# de Venta haga cumplir `total == cantidad * precio_unitario`. Acá va sólo la
# forma de los campos; el invariante de consistencia (un `check` block de
# Nickel) lo agrega el dueño del dominio cuando defina `precio_unitario` como
# campo persistido o derive el total dentro del contract.
{
  # Una venta cerrada. La crea `vender`. El script garantiza total coherente.
  Venta = {
    cantidad | Number,
    total | Number,
    currency | String,
  },
}
