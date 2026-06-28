// mover_oportunidad — avanza una Oportunidad por el pipeline de ventas.
// Input ligado: `oportunidad` (entity Oportunidad). Param: etapa (destino).
//
// Pipeline en orden estricto. El dueño del dominio ajusta esta lista (y las
// reglas de abajo) a su proceso comercial real. "perdida" NO está en esta
// lista: es el otro desenlace terminal del embudo y se alcanza por su propio
// morfismo (`marcar_perdida`), que exige un motivo. Este morfismo sólo avanza
// por el embudo y cierra en "ganada".
let orden = ["prospecto", "calificado", "propuesta", "negociacion", "ganada"];

let actual = input.states.oportunidad.etapa;
let destino = input.params.etapa;

let i_actual = orden.index_of(actual);
let i_destino = orden.index_of(destino);

// 1) El destino debe ser una etapa conocida del embudo.
if i_destino < 0 {
    throw "etapa destino desconocida: " + destino;
}
// 2) Una oportunidad cerrada (ganada o perdida) ya no se mueve.
if actual == "ganada" || actual == "perdida" {
    throw "oportunidad cerrada (" + actual + "): no admite más movimientos";
}
// 3) No se retrocede en el pipeline (ni se queda en la misma etapa).
if i_destino <= i_actual {
    throw "retroceso no permitido en el pipeline: " + actual + " -> " + destino;
}

[
    #{ op: "set",
       path: #{ entity: "Oportunidad", id: input.ids.oportunidad, field: "etapa" },
       value: destino },
]
