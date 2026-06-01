// registrar_interaccion — registra un contacto con un Cliente.
// Input ligado: `cliente` (entity Cliente). Params: interaccion_id, canal,
// nota, timestamp.
//
// Regla de negocio: el canal debe ser uno de los soportados. El dueño del
// dominio define el catálogo real de canales aquí (o lo mueve a un seed/tabla
// si quiere editarlo sin tocar el script).
let canales = ["llamada", "email", "reunion", "visita", "whatsapp"];
if canales.index_of(input.params.canal) < 0 {
    throw "canal inválido: " + input.params.canal;
}

[
    #{ op: "create", entity: "Interaccion", id: input.params.interaccion_id,
       data: #{
           id: input.params.interaccion_id,
           cliente_id: input.ids.cliente,
           canal: input.params.canal,
           nota: input.params.nota,
           timestamp: input.params.timestamp,
       } },
]
