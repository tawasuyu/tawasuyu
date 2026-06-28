// marcar_perdida — cierra una Oportunidad como "perdida", el segundo
// desenlace terminal del embudo (el otro es "ganada", vía mover_oportunidad).
// Input ligado: `oportunidad` (entity Oportunidad). Param: motivo.
//
// Regla de negocio (paridad con Odoo `crm.lead`): cerrar como perdida exige
// un MOTIVO no vacío — el dato comercial que alimenta el análisis de pérdidas
// (precio, competencia, sin presupuesto…). Una oportunidad ya cerrada (ganada
// o perdida) no se vuelve a cerrar.
let actual = input.states.oportunidad.etapa;

// 1) No se cierra dos veces.
if actual == "ganada" || actual == "perdida" {
    throw "oportunidad cerrada (" + actual + "): no se puede marcar perdida";
}

// 2) El motivo es obligatorio y no vacío.
let motivo = input.params.motivo;
if motivo == () || motivo == "" {
    throw "marcar perdida exige un motivo no vacío";
}

[
    #{ op: "set",
       path: #{ entity: "Oportunidad", id: input.ids.oportunidad, field: "etapa" },
       value: "perdida" },
    #{ op: "set",
       path: #{ entity: "Oportunidad", id: input.ids.oportunidad, field: "motivo" },
       value: motivo },
]
