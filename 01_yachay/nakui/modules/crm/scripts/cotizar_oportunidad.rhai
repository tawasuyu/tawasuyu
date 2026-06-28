// cotizar_oportunidad — materializa una Oportunidad GANADA en una Cotización
// (presupuesto) en estado "borrador". Es el primer tramo del puente comercial
// CRM → facturación: el negocio cerrado se vuelve un documento cotizable, que
// luego `facturar_cotizacion` convierte en una Factura asentada.
// Input ligado: `oportunidad` (entity Oportunidad). Param: cotizacion_id.
//
// Regla de negocio: sólo se cotiza lo que se ganó. Una oportunidad abierta o
// perdida no genera cotización.
let etapa = input.states.oportunidad.etapa;
if etapa != "ganada" {
    throw "sólo se cotiza una oportunidad ganada (etapa actual: " + etapa + ")";
}

[
    #{ op: "create", entity: "Cotizacion", id: input.params.cotizacion_id,
       data: #{
           id: input.params.cotizacion_id,
           oportunidad_id: input.ids.oportunidad,
           cliente_id: input.states.oportunidad.cliente_id,
           titulo: input.states.oportunidad.titulo,
           monto: input.states.oportunidad.monto,
           currency: input.states.oportunidad.currency,
           estado: "borrador",
       } },
]
