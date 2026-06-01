# Esquema de entidades del módulo CRM (Nickel contracts).
#
# Regla aprendida del módulo `treasury`: declarar SÓLO los campos que TODOS
# los records de una entidad traen siempre. Los records llevan también `id`
# (y otros extras): los contracts de record de Nickel son abiertos por
# defecto, así que los campos extra se aceptan; lo que rebota es un campo
# REQUERIDO ausente. No declares aquí un campo que algún seed/test no provee.
{
  # Un cliente del CRM. Sembrado por los tests con {id, nombre, email, empresa}.
  Cliente = {
    nombre | String,
    email | String,
    empresa | String,
    ..
  },

  # Una oportunidad de venta. La crea `abrir_oportunidad` en etapa "prospecto"
  # y la mueve `mover_oportunidad` por el pipeline.
  Oportunidad = {
    cliente_id | String,
    titulo | String,
    monto | Number,
    currency | String,
    etapa | String,
    ..
  },

  # Un toque/contacto con un cliente (llamada, email, reunión…).
  Interaccion = {
    cliente_id | String,
    canal | String,
    nota | String,
    timestamp | String,
    ..
  },
}
