// registrar_cliente — alta de un Cliente con verificación del DÍGITO
// VERIFICADOR del documento fiscal (lo que el regex de Nickel no puede hacer:
// la aritmética módulo 11 / módulo 10 sobre los dígitos).
//
// Reparto de responsabilidades:
//   · FORMA del documento (cédula/RIF/CUIT/RUT…) → contrato Nickel
//     (modules/_fiscal/documentos.ncl), verificado por el kernel en el
//     post-check de toda entidad creada (executor.rs paso 8).
//   · DÍGITO VERIFICADOR → este morfismo. Si no cuadra, `throw` y el alta
//     no ocurre.
//
// Países con verificación de DV implementada y verificada contra vectores
// públicos: VE (RIF), CL (RUT), AR (CUIT), EC (cédula). Los demás países del
// pack pasan sólo por la validación de forma (Nickel); devolvemos `true`
// (no bloqueamos) hasta cargar su algoritmo.
//
// El engine Rhai está sandboxeado (sin to_upper ni str.len()): trabajamos
// sobre arrays de enteros y resolvemos may/min en las comparaciones.

// Dígitos de una cadena como array de enteros (ignora puntos, guiones, letras).
fn digitos(s) {
    let out = [];
    for c in s.chars() {
        if c >= '0' && c <= '9' { out.push(c.to_int() - 48); }
    }
    out
}

// Primera letra de tipo de un RIF venezolano (V/E/J/P/G), en mayúscula, o ""
// si el documento arranca con dígito (cédula sin letra de tipo).
fn letra_tipo_ve(s) {
    for c in s.chars() {
        if c == 'V' || c == 'v' { return "V"; }
        if c == 'E' || c == 'e' { return "E"; }
        if c == 'J' || c == 'j' { return "J"; }
        if c == 'P' || c == 'p' { return "P"; }
        if c == 'G' || c == 'g' { return "G"; }
        if c >= '0' && c <= '9' { return ""; }
    }
    ""
}

// ── Chile · RUT · módulo 11 ────────────────────────────────────────────────
// Pesos 2..7 cíclicos de derecha a izquierda sobre el cuerpo; DV = 11 − (Σ%11),
// con 11→0 y 10→K. Vector verificado: 27.962.409-2 (DV 2).
fn dv_ok_cl(doc) {
    // Tokens: dígitos como int, K/k como 10 (sólo válido como DV final).
    let t = [];
    for c in doc.chars() {
        if c >= '0' && c <= '9' { t.push(c.to_int() - 48); }
        else if c == 'K' || c == 'k' { t.push(10); }
    }
    let n = t.len();
    if n < 2 { return false; }
    let dv_dado = t[n - 1];
    let pesos = [2, 3, 4, 5, 6, 7];
    let suma = 0;
    let i = 0;
    while i < n - 1 {
        suma += t[n - 2 - i] * pesos[i % 6];
        i += 1;
    }
    let r = 11 - (suma % 11);
    let dv = r;
    if r == 11 { dv = 0; }
    if r == 10 { dv = 10; }
    dv == dv_dado
}

// ── Argentina · CUIT/CUIL · módulo 11 ──────────────────────────────────────
// Pesos [5,4,3,2,7,6,5,4,3,2] sobre los 10 primeros; DV = 11 − (Σ%11), 11→0,
// 10→inválido. Vector verificado: 20-17254359-7 (DV 7).
fn dv_ok_ar(doc) {
    let ds = digitos(doc);
    if ds.len() != 11 { return true; }
    let pesos = [5, 4, 3, 2, 7, 6, 5, 4, 3, 2];
    let suma = 0;
    let i = 0;
    while i < 10 {
        suma += ds[i] * pesos[i];
        i += 1;
    }
    let r = 11 - (suma % 11);
    if r == 10 { return false; }
    let dv = r;
    if r == 11 { dv = 0; }
    dv == ds[10]
}

// ── Ecuador · cédula · módulo 10 ───────────────────────────────────────────
// Coeficientes [2,1,…] sobre los 9 primeros; producto ≥10 resta 9; DV =
// (10 − Σ%10) % 10. Vector verificado: 1710034065 (DV 5). RUC de 13 dígitos:
// no verificamos DV acá (otra fórmula) → pasa por forma.
fn dv_ok_ec(doc) {
    let ds = digitos(doc);
    if ds.len() != 10 { return true; }
    let coef = [2, 1, 2, 1, 2, 1, 2, 1, 2];
    let suma = 0;
    let i = 0;
    while i < 9 {
        let p = ds[i] * coef[i];
        if p > 9 { p -= 9; }
        suma += p;
        i += 1;
    }
    let dv = (10 - (suma % 10)) % 10;
    dv == ds[9]
}

// ── Venezuela · RIF · módulo 11 ────────────────────────────────────────────
// base(letra): V=4 E=8 J=12 P=16 G=20; pesos [3,2,7,6,5,4,3,2] sobre los 8
// dígitos de cuerpo; DV = 11 − (Σ%11), con DV≥10 → 0. Vector verificado:
// G-20009048-0 (DV 0). Cédula (letra + ≤8 dígitos, sin DV) → no aplica.
fn dv_ok_ve(doc) {
    let ds = digitos(doc);
    if ds.len() != 9 { return true; } // sólo el RIF (8 cuerpo + 1 DV) tiene DV
    let letra = letra_tipo_ve(doc);
    let base = 0;
    if letra == "V" { base = 4; }
    else if letra == "E" { base = 8; }
    else if letra == "J" { base = 12; }
    else if letra == "P" { base = 16; }
    else if letra == "G" { base = 20; }
    else { return true; }
    let pesos = [3, 2, 7, 6, 5, 4, 3, 2];
    let suma = base;
    let i = 0;
    while i < 8 {
        suma += ds[i] * pesos[i];
        i += 1;
    }
    let r = 11 - (suma % 11);
    let dv = r;
    if r >= 10 { dv = 0; }
    dv == ds[8]
}

let pais = input.params.pais;
let doc = input.params.documento;

let ok = true;
if pais == "CL" { ok = dv_ok_cl(doc); }
else if pais == "AR" { ok = dv_ok_ar(doc); }
else if pais == "EC" { ok = dv_ok_ec(doc); }
else if pais == "VE" { ok = dv_ok_ve(doc); }

if !ok {
    throw "documento inválido para " + pais + ": el dígito verificador no coincide (" + doc + ")";
}

[
    #{ op: "create", entity: "Cliente", id: input.params.id,
       data: #{
           id: input.params.id,
           nombre: input.params.nombre,
           email: input.params.email,
           empresa: input.params.empresa,
           pais: pais,
           documento: doc,
       } },
]
