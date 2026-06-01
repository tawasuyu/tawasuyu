// mover_oportunidad — avanza una Oportunidad por el pipeline de ventas.
// Input ligado: `oportunidad` (entity Oportunidad). Param: etapa (destino).
//
// Pipeline en orden estricto. El dueño del dominio ajusta esta lista (y las
// reglas de abajo) a su proceso comercial real:
let orden = ["prospecto", "calificado", "propuesta", "negociacion", "ganada"];

let actual = input.states.oportunidad.etapa;
let destino = input.params.etapa;

let i_actual = orden.index_of(actual);
let i_destino = orden.index_of(destino);

// 1) El destino debe ser una etapa conocida.
if i_destino < 0 {
    throw "etapa destino desconocida: " + destino;
}
// 2) "ganada" es terminal: una oportunidad cerrada ya no se mueve.
if actual == "ganada" {
    throw "oportunidad cerrada (ganada): no admite más movimientos";
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
