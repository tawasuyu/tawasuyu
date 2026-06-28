// asentar_n — asiento de PARTIDA DOBLE de N patas. Liga N cuentas como
// rol VARIÁDICO `lineas` (entity Cuenta); `params.movimientos` es un
// array PARALELO de `{ debe, haber }` por línea y `params.renglon_ids`
// los UUID (idempotencia) de las líneas. Params: movimientos,
// renglon_ids, glosa, fecha, diario, comprobante_id.
//
// Por cada línea aplica, en convención deudor-normal,
//     saldo += debe − haber
// de modo que Σ Δ saldo = Σ (debe − haber). La regla `conserve`
// (Cuenta.saldo agrupado por moneda) exige que ese total sea 0: ESA es
// la partida doble de N patas, garantizada por el kernel sobre el
// conjunto variádico, no por este script. Un comprobante desbalanceado
// (Σ debe ≠ Σ haber) lo rebota el executor con ConservationViolation.
//
// Modelo cabecera + líneas: un `Comprobante` (total) y un `Renglon` plano
// por pata. El executor ya rechaza ligar la misma cuenta dos veces
// (DuplicateInputId) y montos que dejen `Comprobante.monto` negativo o un
// `Renglon` con debe/haber negativo (post-check NoNegativo).
let n = input.ids.lineas.len();
let movs = input.params.movimientos;
let rids = input.params.renglon_ids;
if movs.len() != n {
    throw "movimientos (" + movs.len() + ") no coincide con líneas (" + n + ")";
}
if rids.len() != n {
    throw "renglon_ids (" + rids.len() + ") no coincide con líneas (" + n + ")";
}
if n < 2 {
    throw "un asiento necesita al menos 2 patas, recibí " + n;
}

let ops = [];
let total_debe = 0;
for i in 0..n {
    let cuenta_id = input.ids.lineas[i];
    let estado = input.states.lineas[i];
    let debe = movs[i].debe;
    let haber = movs[i].haber;
    ops.push(#{ op: "set",
        path: #{ entity: "Cuenta", id: cuenta_id, field: "saldo" },
        value: estado.saldo + debe - haber });
    ops.push(#{ op: "create", entity: "Renglon", id: rids[i],
        data: #{
            id: rids[i],
            comprobante_id: input.params.comprobante_id,
            cuenta_id: cuenta_id,
            debe: debe,
            haber: haber,
        } });
    total_debe += debe;
}

ops.push(#{ op: "create", entity: "Comprobante", id: input.params.comprobante_id,
    data: #{
        id: input.params.comprobante_id,
        fecha: input.params.fecha,
        glosa: input.params.glosa,
        diario: input.params.diario,
        monto: total_debe,
        moneda: input.states.lineas[0].moneda,
    } });

ops
