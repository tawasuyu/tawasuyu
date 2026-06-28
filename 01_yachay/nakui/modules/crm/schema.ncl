# Esquema de entidades del módulo CRM (Nickel contracts).
#
# Regla aprendida del módulo `treasury`: declarar SÓLO los campos que TODOS
# los records de una entidad traen siempre. Los records llevan también `id`
# (y otros extras): los contracts de record de Nickel son abiertos por
# defecto, así que los campos extra se aceptan; lo que rebota es un campo
# REQUERIDO ausente. No declares aquí un campo que algún seed/test no provee.
#
# Cliente importa el pack fiscal compartido (`../_fiscal/documentos.ncl`) para
# validar el documento tributario según el país, SIN forkear el módulo por
# país. Es opt-in por registro: un cliente sin `documento` sigue siendo
# válido (retrocompat con los seeds {id,nombre,email,empresa}); si trae
# `documento`, debe traer `pais` y respetar el formato de ese país.
let fiscal = import "../_fiscal/documentos.ncl" in
{
  # Un cliente del CRM. Sembrado por los tests con {id, nombre, email, empresa};
  # opcionalmente {pais, documento} para validación fiscal (cédula/RIF/CUIT…).
  Cliente = std.contract.Sequence [
    fiscal.ConDocumentoFiscal,
    {
      nombre | String,
      email | String,
      empresa | String,
      ..
    },
  ],

  # Una oportunidad de venta. La crea `abrir_oportunidad` en etapa "prospecto",
  # la mueve `mover_oportunidad` por el embudo hasta cerrar en "ganada", y
  # `marcar_perdida` la cierra como "perdida" agregando un `motivo` (campo
  # opcional, presente sólo tras el cierre-perdida; el record es abierto).
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
