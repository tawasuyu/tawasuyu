# Esquema de entidades del módulo Inventory (Nickel contracts).
#
# `cantidad` usa un contract no-negativo: el post-check KCL del kernel rebota
# cualquier op que deje un Stock en negativo (sobregiro). Ese es el mecanismo
# por el que `transferir`/`vender` con cantidad excesiva fallan con SchemaPost
# en vez de corromper el stock. Idiom tomado de los tests de nickel_validator.
let NoNegativo = std.contract.from_predicate (fun n => std.is_number n && n >= 0) in
{
  # Unidades de un SKU en una ubicación. Sembrado por los tests con
  # {id, sku_id, ubicacion, cantidad}.
  Stock = {
    sku_id | String,
    ubicacion | String,
    cantidad | NoNegativo,
    ..
  },

  # Registro append-only de un movimiento de stock (recepción/transferencia).
  MovimientoStock = {
    cantidad | Number,
    ..
  },
}
