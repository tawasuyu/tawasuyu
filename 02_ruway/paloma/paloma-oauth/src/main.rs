//! `paloma-oauth` — el helper de **autorización OAuth2** del correo.
//!
//! `paloma` no puede pedirle al usuario su contraseña de Google/Microsoft: esos
//! proveedores cerraron IMAP/SMTP a las contraseñas y exigen **OAuth2**. Este
//! binario hace el flujo de escritorio recomendado —*Authorization Code* con
//! **PKCE** por *loopback*—: levanta un servidor en `127.0.0.1`, abre el
//! navegador en el proveedor, recibe el código de vuelta, lo cambia por un
//! `access_token` + `refresh_token` y los guarda en `oauth-<id>.json` (0600).
//! `paloma-app` lee de ahí el `access_token` y autentica por `XOAUTH2`.
//!
//! Re-ejecutarlo cuando ya hay un `refresh_token` **renueva sin navegador**
//! (útil cuando el access token venció, ~1 h).
//!
//! ## Uso
//!
//! ```bash
//! paloma-oauth <id-de-cuenta>     # autoriza/renueva la cuenta de cuentas.json
//! paloma-oauth <id> --force       # fuerza el flujo del navegador (re-consentir)
//! ```
//!
//! Requisito: la cuenta (en `cuentas.json`) debe ser `auth: oauth2` con un
//! `oauth_provider` (google/microsoft) y un **`oauth_client_id`** de una app
//! OAuth registrada por vos en el proveedor (las de escritorio usan PKCE; el
//! `client_secret` queda vacío salvo que el proveedor lo exija).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::ExitCode;

use base64::Engine;
use directories::ProjectDirs;
use paloma_config::{oauth_token_path, AccountEntry, PalomaConfig, Preset};
use paloma_oauth::{existing_refresh_token, refresh, save_token, token_request, Token};
use sha2::{Digest, Sha256};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let force = args.iter().any(|a| a == "--force");
    let id = match args.iter().skip(1).find(|a| !a.starts_with("--")) {
        Some(id) => id.clone(),
        None => {
            eprintln!("uso: paloma-oauth <id-de-cuenta> [--force]");
            return ExitCode::FAILURE;
        }
    };
    match run(&id, force) {
        Ok(msg) => {
            println!("✓ {msg}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("✗ {e}");
            ExitCode::FAILURE
        }
    }
}

/// El directorio de config de paloma (`~/.config/paloma`).
fn config_dir() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("PALOMA_CONFIG") {
        if let Some(parent) = PathBuf::from(p).parent() {
            return Ok(parent.to_path_buf());
        }
    }
    ProjectDirs::from("org", "tawasuyu", "paloma")
        .map(|d| d.config_dir().to_path_buf())
        .ok_or_else(|| "no se pudo resolver el dir de config de paloma".to_string())
}

fn run(id: &str, force: bool) -> Result<String, String> {
    let dir = config_dir()?;
    let cfg = PalomaConfig::load(&paloma_config::config_path(&dir))
        .map_err(|e| format!("config inválida: {e}"))?;
    let entry = cfg.get(id).ok_or_else(|| format!("no existe la cuenta «{id}» en cuentas.json"))?;
    if !entry.is_oauth() {
        return Err(format!("la cuenta «{id}» no usa OAuth2 (auth != oauth2)"));
    }
    let preset = entry
        .oauth_preset()
        .ok_or_else(|| format!("proveedor OAuth desconocido: «{}»", entry.oauth_provider))?;
    if entry.oauth_client_id.trim().is_empty() {
        print_setup_guide(id, entry, preset);
        return Err(format!("falta oauth_client_id de «{id}» (ver los pasos de arriba)"));
    }
    let token_path = oauth_token_path(&dir, id);

    // Camino rápido: si ya hay refresh_token y no se fuerza, renová sin navegador.
    if !force {
        if let Some(rt) = existing_refresh_token(&token_path) {
            match refresh(entry, preset, &rt) {
                Ok(tok) => {
                    let tok = tok.with_refresh_fallback(&rt);
                    save_token(&token_path, &tok)?;
                    return Ok(format!("token de «{id}» renovado (sin navegador)"));
                }
                Err(e) => eprintln!("· no se pudo renovar ({e}); voy al navegador…"),
            }
        }
    }

    // Flujo completo: PKCE + loopback + navegador.
    let tok = authorize(entry, preset)?;
    save_token(&token_path, &tok)?;
    Ok(format!("cuenta «{id}» autorizada — token en {}", token_path.display()))
}

/// Imprime una guía paso a paso para registrar la app OAuth y conseguir el
/// `client_id` (lo único que el usuario tiene que hacer a mano: Google/Microsoft
/// no permiten clientes genéricos). Específica por proveedor.
fn print_setup_guide(id: &str, entry: &AccountEntry, preset: &Preset) {
    eprintln!(
        "\n── Configurar OAuth para «{id}» ({}) ──\n\
         Falta el client_id. Es lo único manual: {} exige que registres tu propia\n\
         app OAuth (no hay clientes genéricos). Una sola vez:\n",
        entry.email, preset.label
    );
    match entry.oauth_provider.as_str() {
        "google" => eprintln!(
            " 1. https://console.cloud.google.com/apis/credentials  (creá/elegí un proyecto)\n\
             2. Habilitá la «Gmail API» (APIs y servicios → Biblioteca).\n\
             3. Configurá la «Pantalla de consentimiento OAuth» (tipo Externo) y agregá tu\n\
             \x20   correo como «usuario de prueba».\n\
             4. Credenciales → «Crear credenciales» → «ID de cliente de OAuth» →\n\
             \x20   tipo de aplicación: «Aplicación de escritorio».\n\
             5. Copiá el «ID de cliente» (con PKCE el client_secret queda vacío)."
        ),
        "microsoft" => eprintln!(
            " 1. https://entra.microsoft.com → «App registrations» → «New registration».\n\
             2. Platform: «Mobile and desktop applications», redirect URI «http://localhost».\n\
             3. En «Authentication», poné «Allow public client flows» = Yes (PKCE, sin secret).\n\
             4. «API permissions» → Office 365 Exchange Online (delegado):\n\
             \x20   IMAP.AccessAsUser.All y SMTP.Send.\n\
             5. Copiá el «Application (client) ID»."
        ),
        other => eprintln!(
            " El proveedor «{other}» no tiene preset. Registrá tu app OAuth en él como\n\
             cliente público/escritorio con PKCE y un redirect de loopback, y completá\n\
             oauth_provider/auth_url/token_url/scope a mano."
        ),
    }
    eprintln!(
        "\n 6. Pegá el client_id en el panel (diente Correo → OAuth → client_id) o en\n\
         \x20   {}  (campo «oauth_client_id»).\n\
         7. Reintentá:  paloma-oauth {id}\n\n\
         Detalles del flujo: redirect en http://127.0.0.1:<puerto-libre> (loopback;\n\
         las apps de escritorio lo permiten). Scope que se pedirá:\n\
         \x20  {}\n",
        paloma_config::config_path(&config_dir().unwrap_or_default()).display(),
        preset.scope,
    );
}

// =====================================================================
// El flujo OAuth2 (interactivo) — la parte no interactiva (token/refresh) vive
// en la lib (`lib.rs`), compartida con paloma-app.
// =====================================================================

/// Corre el flujo Authorization Code + PKCE por loopback y devuelve el token.
fn authorize(entry: &AccountEntry, preset: &Preset) -> Result<Token, String> {
    // 1) Listener loopback en un puerto efímero (el SO elige el libre).
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("bind loopback: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}");

    // 2) PKCE + state anti-CSRF.
    let verifier = random_url_token(64);
    let challenge = pkce_challenge(&verifier);
    let state = random_url_token(24);

    // 3) URL de autorización. `access_type=offline` + `prompt=consent` (Google)
    //    fuerzan a que devuelva refresh_token la primera vez.
    let auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}\
         &code_challenge={}&code_challenge_method=S256&access_type=offline&prompt=consent",
        preset.auth_url,
        urlencode(&entry.oauth_client_id),
        urlencode(&redirect_uri),
        urlencode(preset.scope),
        urlencode(&state),
        challenge,
    );

    println!("Abriendo el navegador para autorizar «{}»…", entry.email);
    println!("Si no abre solo, pegá esta URL en tu navegador:\n{auth_url}\n");
    open_browser(&auth_url);

    // 4) Esperamos el redirect con el código.
    let (code, got_state) = wait_for_code(&listener)?;
    if got_state != state {
        return Err("state no coincide (posible CSRF) — abortado".to_string());
    }

    // 5) Cambiamos el código por el token.
    let mut form: Vec<(&str, &str)> = vec![
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", &redirect_uri),
        ("client_id", &entry.oauth_client_id),
        ("code_verifier", &verifier),
    ];
    if !entry.oauth_client_secret.trim().is_empty() {
        form.push(("client_secret", &entry.oauth_client_secret));
    }
    token_request(preset, &form)
}

/// Bloquea hasta recibir el redirect del proveedor y devuelve `(code, state)`.
/// Responde una página mínima al navegador para que el usuario sepa que terminó.
fn wait_for_code(listener: &TcpListener) -> Result<(String, String), String> {
    for stream in listener.incoming() {
        let mut stream = stream.map_err(|e| format!("accept: {e}"))?;
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).map_err(|e| format!("read: {e}"))?;
        let req = String::from_utf8_lossy(&buf[..n]);
        // Primera línea: `GET /?code=...&state=... HTTP/1.1`.
        let target = req.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("");
        let params = parse_query(target);
        if let Some(err) = params.iter().find(|(k, _)| k == "error").map(|(_, v)| v.clone()) {
            respond(&mut stream, "Autorización rechazada. Podés cerrar esta pestaña.");
            return Err(format!("el proveedor devolvió error: {err}"));
        }
        let code = params.iter().find(|(k, _)| k == "code").map(|(_, v)| v.clone());
        let state = params.iter().find(|(k, _)| k == "state").map(|(_, v)| v.clone());
        match (code, state) {
            (Some(code), Some(state)) => {
                respond(&mut stream, "✓ paloma autorizada. Ya podés cerrar esta pestaña y volver al correo.");
                return Ok((code, state));
            }
            _ => {
                // Pedidos sueltos (favicon, etc.): respondemos y seguimos esperando.
                respond(&mut stream, "Esperando la autorización…");
            }
        }
    }
    Err("el listener se cerró sin recibir el código".to_string())
}

/// Responde un 200 con un cuerpo HTML mínimo y cierra.
fn respond(stream: &mut std::net::TcpStream, body: &str) {
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>paloma</title>\
         <body style='font-family:sans-serif;background:#11131a;color:#e6e6e6;\
         display:flex;align-items:center;justify-content:center;height:100vh'>\
         <p style='font-size:1.2rem'>{body}</p>"
    );
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    let _ = stream.write_all(resp.as_bytes());
}

// =====================================================================
// Utilidades
// =====================================================================

/// Abre `url` en el navegador del sistema (best-effort: `xdg-open`).
fn open_browser(url: &str) {
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

/// El `code_challenge` PKCE = base64url(sha256(verifier)) sin padding (S256).
fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// Token aleatorio URL-safe de `len` caracteres (verifier/state).
fn random_url_token(len: usize) -> String {
    use rand::Rng;
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut rng = rand::thread_rng();
    (0..len).map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char).collect()
}

/// Percent-encoding mínimo para los parámetros de la URL de autorización.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Decodifica `%XX` y `+` de un valor de query.
fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    out.push(v);
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parsea el query string de un target `/?k=v&k2=v2` a pares decodificados.
fn parse_query(target: &str) -> Vec<(String, String)> {
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((urldecode(k), urldecode(v)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_rfc7636_ejemplo() {
        // Vector del RFC 7636 (Apéndice B).
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(pkce_challenge(verifier), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn parse_query_decodifica_pares() {
        let q = parse_query("/?code=4%2F0Ab&state=xy_z&scope=https%3A%2F%2Fmail");
        assert_eq!(q.iter().find(|(k, _)| k == "code").unwrap().1, "4/0Ab");
        assert_eq!(q.iter().find(|(k, _)| k == "state").unwrap().1, "xy_z");
        assert_eq!(q.iter().find(|(k, _)| k == "scope").unwrap().1, "https://mail");
    }

    #[test]
    fn urlencode_preserva_unreserved() {
        assert_eq!(urlencode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(urlencode("a b/c"), "a%20b%2Fc");
    }
}
