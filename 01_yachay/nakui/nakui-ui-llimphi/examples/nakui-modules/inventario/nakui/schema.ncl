# Schema autocontenido del módulo UI Inventario: Cuenta (copia de
# accounting) + Producto + MovimientoStock. La store del backend es
# compartida, así que las Cuenta sembradas por `contabilidad` son las que
# eligen los forms de recibir/despachar.
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

  Producto = {
    sku | String,
    nombre | String,
    cantidad | NoNegativo,
    valor_total | NoNegativo,
    moneda | String,
    ..
  },

  MovimientoStock = {
    producto_id | String,
    tipo | String,
    cantidad | NoNegativo,
    costo_total | NoNegativo,
    ..
  },
}
