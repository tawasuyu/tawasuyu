// Módulo Rhai COMPARTIDO de dígito verificador fiscal. Lo registra el
// RhaiExecutor (nakui-core) como módulo estático "fiscal" vía
// StaticModuleResolver; cualquier morfismo lo usa con:
//
//     import "fiscal" as f;
//     if !f::dv_ok(pais, documento) { throw "DV inválido"; }
//
// Punto de entrada único: `dv_ok(pais, doc) -> bool`. Devuelve true si el
// dígito verificador cuadra, si el país no tiene algoritmo cargado, o si el
// documento no lleva DV (cédula). false sólo cuando hay regla y NO cuadra.
//
// Reparto con el pack: la FORMA la valida el contrato Nickel
// (_fiscal/documentos.ncl) en el post-check del kernel; acá va sólo la
// ARITMÉTICA módulo 11/10 que el regex no puede hacer.
//
// Algoritmos verificados contra vectores públicos reales (ver cada fn). El
// engine Rhai está sandboxeado (sin to_upper ni str.len()): todo sobre
// arrays de enteros.

// Dígitos de una cadena como array de enteros (ignora puntos, guiones, letras).
fn digitos(s) {
    let out = [];
    for c in s.chars() {
        if c >= '0' && c <= '9' { out.push(c.to_int() - 48); }
    }
    out
}

// Primera letra de tipo de un RIF venezolano (V/E/J/P/G) en mayúscula, o ""
// si arranca con dígito (cédula sin letra de tipo).
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

// Chile · RUT · módulo 11. Pesos 2..7 cíclicos de derecha a izquierda; DV =
// 11 − (Σ%11), con 11→0 y 10→K. Vector verificado: 27.962.409-2 (DV 2).
fn dv_ok_cl(doc) {
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

// Argentina · CUIT/CUIL · módulo 11. Pesos [5,4,3,2,7,6,5,4,3,2] sobre los 10
// primeros; DV = 11 − (Σ%11), 11→0, 10→inválido. Verificado: 20-17254359-7.
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

// Ecuador · cédula · módulo 10. Coeficientes [2,1,…] sobre los 9 primeros;
// producto ≥10 resta 9; DV = (10 − Σ%10) % 10. Verificado: 1710034065 (DV 5).
// RUC de 13 dígitos: otra fórmula → no verificamos DV acá.
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

// Venezuela · RIF · módulo 11. base(letra) V=4 E=8 J=12 P=16 G=20; pesos
// [3,2,7,6,5,4,3,2] sobre los 8 dígitos de cuerpo; DV = 11 − (Σ%11), DV≥10→0.
// Verificado: G-20009048-0 (DV 0). Cédula (letra + ≤8 dígitos): no aplica.
fn dv_ok_ve(doc) {
    let ds = digitos(doc);
    if ds.len() != 9 { return true; }
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

// Dispatch por país. Países sin algoritmo cargado → true (sólo forma).
fn dv_ok(pais, doc) {
    if pais == "CL" { return dv_ok_cl(doc); }
    if pais == "AR" { return dv_ok_ar(doc); }
    if pais == "EC" { return dv_ok_ec(doc); }
    if pais == "VE" { return dv_ok_ve(doc); }
    true
}
