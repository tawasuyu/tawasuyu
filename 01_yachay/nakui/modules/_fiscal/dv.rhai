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
// DV implementado y verificado contra vectores públicos reales: VE (RIF),
// CL (RUT), AR (CUIT), EC (cédula), PE (RUC), CO (NIT), MX (RFC). BO queda
// fuera: su NIT no tiene dígito de control con fórmula pública (ver dv_ok).
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

// Perú · RUC · módulo 11. Pesos [5,4,3,2,7,6,5,4,3,2] sobre los 10 primeros;
// DV = 11 − (Σ%11), 10→0, 11→1. Verificado: 20306051017 (DV 7). El DNI (8
// dígitos) no lleva DV → pasa por forma.
fn dv_ok_pe(doc) {
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
    let dv = r;
    if r == 10 { dv = 0; }
    if r == 11 { dv = 1; }
    dv == ds[10]
}

// Colombia · NIT · módulo 11 con primos. Sólo valida si el documento trae el
// DV separado por guion ("800197268-4"); sin guion lo tratamos como cédula
// (sin DV) y pasa. Primos de derecha a izquierda; resto 0→0, 1→1, else
// 11−resto. Verificado: 800197268-4 (DV 4).
fn dv_ok_co(doc) {
    let ds = [];
    let dash_at = -1; // cuántos dígitos había al aparecer el guion
    for c in doc.chars() {
        if c >= '0' && c <= '9' { ds.push(c.to_int() - 48); }
        else if c == '-' { dash_at = ds.len(); }
    }
    // DV presente sólo si hubo guion y exactamente un dígito después.
    if dash_at < 0 || ds.len() != dash_at + 1 { return true; }
    let base_n = dash_at;
    let dv_dado = ds[base_n];
    let primos = [3, 7, 13, 17, 19, 23, 29, 37, 41, 43, 47, 53, 59, 67, 71];
    let suma = 0;
    let i = 0;
    while i < base_n {
        suma += ds[base_n - 1 - i] * primos[i];
        i += 1;
    }
    let resto = suma % 11;
    let dv = resto;
    if resto > 1 { dv = 11 - resto; }
    dv == dv_dado
}

// Valor de un carácter en la tabla del SAT (Anexo 20) para el DV del RFC.
// 0-9→0-9; A-N→10-23; &→24; O-Z→25-36; espacio→37; Ñ→38. -1 si inesperado.
fn valor_rfc(c) {
    let n = c.to_int();
    if c >= '0' && c <= '9' { return n - 48; }
    if c == '&' { return 24; }
    if c == ' ' { return 37; }
    if n == 209 || n == 241 { return 38; } // Ñ / ñ por code-point
    let p = -1;
    if c >= 'A' && c <= 'Z' { p = n - 65; }
    else if c >= 'a' && c <= 'z' { p = n - 97; }
    if p < 0 { return -1; }
    if p <= 13 { return 10 + p; } // A..N
    11 + p                        // O..Z (corrige el slot de &)
}

// México · RFC · módulo 11. DV = último carácter, sobre los 12 previos (los
// 11 de un RFC moral se rellenan a la izquierda con un espacio). Pesos
// 13..2; DV = 11 − (Σ%11), con 11→'0' y 10→'A'. Verificado: GODE561231GR8.
fn dv_ok_mx(doc) {
    let chars = [];
    for c in doc.chars() {
        let n = c.to_int();
        if (c >= '0' && c <= '9') || (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z')
           || c == '&' || n == 209 || n == 241 {
            chars.push(c);
        }
    }
    let n = chars.len();
    if n != 12 && n != 13 { return true; }
    let dv_char = chars[n - 1];
    let cuerpo_n = n - 1;          // 11 (moral) o 12 (física)
    let pad = 12 - cuerpo_n;       // espacios de relleno a la izquierda
    let suma = 0;
    let pos = 0;
    while pos < pad {
        suma += 37 * (13 - pos);  // espacio = 37
        pos += 1;
    }
    let i = 0;
    while i < cuerpo_n {
        let v = valor_rfc(chars[i]);
        if v < 0 { return true; }
        suma += v * (13 - pos);
        pos += 1;
        i += 1;
    }
    let y = 11 - (suma % 11);
    if y == 11 { return dv_char == '0'; }
    if y == 10 { return dv_char == 'A' || dv_char == 'a'; }
    if dv_char >= '0' && dv_char <= '9' { return (dv_char.to_int() - 48) == y; }
    false
}

// Dispatch por país. Países sin algoritmo cargado → true (sólo forma).
// Bolivia (BO) queda fuera a propósito: su NIT termina en un dígito
// asignado (descrito como aleatorio 1–9), no en un dígito de control con
// fórmula pública verificable. Validar uno inventado sería peor que nada.
fn dv_ok(pais, doc) {
    if pais == "CL" { return dv_ok_cl(doc); }
    if pais == "AR" { return dv_ok_ar(doc); }
    if pais == "EC" { return dv_ok_ec(doc); }
    if pais == "VE" { return dv_ok_ve(doc); }
    if pais == "PE" { return dv_ok_pe(doc); }
    if pais == "CO" { return dv_ok_co(doc); }
    if pais == "MX" { return dv_ok_mx(doc); }
    true
}

// ════════════════════════════════════════════════════════════════════════
// NORMALIZACIÓN — "que el documento salga con cierto formato". Devuelve la
// forma canónica de presentación del documento (la que se guarda). Si no
// reconoce el documento, lo devuelve TAL CUAL: así un valor mal formado no
// se "arregla" silenciosamente y lo rebota el contrato Nickel.
//
// INVARIANTE: la salida canónica DEBE seguir matcheando el regex del país
// en _fiscal/documentos.ncl (el kernel valida la entidad creada). Cada
// formato de abajo se eligió para cumplirlo.

// Substring de un array de dígitos como cadena: ds[desde..hasta].
fn ds_str(ds, desde, hasta) {
    let out = "";
    let i = desde;
    while i < hasta {
        out += ds[i].to_string();
        i += 1;
    }
    out
}

// Agrupa los primeros `n` dígitos de `ds` de a 3 desde la derecha con puntos
// (separador de miles): [1,2,3,4,5,6,7,8],8 → "12.345.678".
fn agrupa_miles(ds, n) {
    let out = "";
    let i = 0;
    while i < n {
        let desde_derecha = n - i;
        if i > 0 && (desde_derecha % 3) == 0 { out += "."; }
        out += ds[i].to_string();
        i += 1;
    }
    out
}

// Venezuela: RIF "J-12345678-9" (8 cuerpo + DV) o cédula "V-12345678".
fn norm_ve(doc) {
    let letra = letra_tipo_ve(doc);
    let ds = digitos(doc);
    let n = ds.len();
    if letra == "" || n < 6 { return doc; }
    if n == 9 { return letra + "-" + ds_str(ds, 0, 8) + "-" + ds[8].to_string(); }
    if n > 9 { return doc; }
    letra + "-" + ds_str(ds, 0, n)
}

// Argentina: CUIT/CUIL "20-12345678-9".
fn norm_ar(doc) {
    let ds = digitos(doc);
    if ds.len() != 11 { return doc; }
    ds_str(ds, 0, 2) + "-" + ds_str(ds, 2, 10) + "-" + ds[10].to_string()
}

// Ecuador: sólo dígitos, sin separadores (cédula 10 / RUC 13).
fn norm_ec(doc) {
    let ds = digitos(doc);
    let n = ds.len();
    if n == 10 || n == 13 { return ds_str(ds, 0, n); }
    doc
}

// Chile: RUT "12.345.678-5" (cuerpo con puntos, guion, DV; K en mayúscula).
fn norm_cl(doc) {
    let cuerpo = [];
    let dv = "";
    for c in doc.chars() {
        if c >= '0' && c <= '9' { cuerpo.push(c.to_int() - 48); }
        else if c == 'K' || c == 'k' { dv = "K"; }
    }
    let n = cuerpo.len();
    if dv == "" {
        if n < 2 { return doc; }
        dv = cuerpo[n - 1].to_string();
        n = n - 1; // el último dígito recolectado era el DV
    }
    if n < 7 || n > 8 { return doc; }
    agrupa_miles(cuerpo, n) + "-" + dv
}

// Dispatch. Países sin forma canónica cargada → devuelven el documento igual.
fn normalizar(pais, doc) {
    if type_of(doc) != "string" { return doc; }
    if pais == "VE" { return norm_ve(doc); }
    if pais == "AR" { return norm_ar(doc); }
    if pais == "EC" { return norm_ec(doc); }
    if pais == "CL" { return norm_cl(doc); }
    doc
}
