// abrir_oportunidad — crea una Oportunidad para un Cliente, en etapa inicial
// "prospecto". Input ligado: `cliente` (entity Cliente). Params esperados:
//   oportunidad_id, titulo, monto, currency, timestamp.
//
// Regla de negocio (la mínima que exige el test): el monto no puede ser
// negativo. Acá es donde el dueño del dominio agrega más reglas (p.ej. monto
// mínimo por moneda, título no vacío, límite de oportunidades abiertas…).
if input.params.monto < 0 {
    throw "monto inválido: una oportunidad no puede abrirse con monto negativo";
}

[
    #{ op: "create", entity: "Oportunidad", id: input.params.oportunidad_id,
       data: #{
           id: input.params.oportunidad_id,
           cliente_id: input.ids.cliente,
           titulo: input.params.titulo,
           monto: input.params.monto,
           currency: input.params.currency,
           etapa: "prospecto",
       } },
]
