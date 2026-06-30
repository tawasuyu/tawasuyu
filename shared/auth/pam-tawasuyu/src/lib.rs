//! `pam_tawasuyu.so` — **desbloqueo automático de la identidad agora al login**.
//!
//! Cierra el último eslabón del cifrado de dotfiles de `pacha` (Fase 3): que la
//! seed de identidad quede desbloqueada en la sesión **sin re-pedir la frase**.
//!
//! ## Cómo funciona
//!
//! - **`pam_sm_authenticate`** (fase `auth`, DESPUÉS de `pam_unix`): lee la
//!   contraseña que `pam_unix` ya validó (`PAM_AUTHTOK`) y la guarda en el
//!   handle PAM para la fase de sesión. No toma ninguna decisión de auth
//!   (devuelve `PAM_IGNORE`).
//! - **`pam_sm_open_session`** (fase `session`, DESPUÉS de `pam_keyinit`):
//!   ejecuta `agora-cli desbloquear` **como el usuario** (drop de uid/gid), con
//!   la frase en `AGORA_PASSPHRASE`. El hijo hereda el *session keyring* que
//!   `pam_keyinit` creó para la sesión; al escribir la seed ahí (como el
//!   usuario), queda en el keyring que la sesión del usuario hereda → `pacha`
//!   la encuentra. `pam_keyinit revoke` la evapora al cerrar sesión.
//!
//! **Best-effort:** nada de esto bloquea el login. Si no hay identidad, si la
//! frase no abre el keystore, o si `agora-cli` no está, el login sigue igual.
//!
//! ## Instalación (resumen; ver `scripts/INSTALACION.md`)
//!
//! ```text
//! # /usr/lib/security/pam_tawasuyu.so  (cdylib renombrado)
//! # en /etc/pam.d/<servicio> (ej. login, o el del greeter):
//! auth     optional  pam_tawasuyu.so
//! session  optional  pam_tawasuyu.so          # tras pam_keyinit.so force revoke
//! # opcional: pam_tawasuyu.so agora_cli=/ruta/a/agora-cli
//! ```

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};

// ── Constantes PAM (Linux-PAM, security/_pam_types.h) ───────────────────────
const PAM_SUCCESS: c_int = 0;
const PAM_AUTHTOK: c_int = 6;
const PAM_IGNORE: c_int = 25;

/// `pam_handle_t` es opaco para nosotros.
type PamHandle = c_void;
type CleanupFn = extern "C" fn(*mut PamHandle, *mut c_void, c_int);

#[link(name = "pam")]
extern "C" {
    fn pam_get_item(pamh: *mut PamHandle, item_type: c_int, item: *mut *const c_void) -> c_int;
    fn pam_get_user(pamh: *mut PamHandle, user: *mut *const c_char, prompt: *const c_char) -> c_int;
    fn pam_set_data(
        pamh: *mut PamHandle,
        module_data_name: *const c_char,
        data: *mut c_void,
        cleanup: Option<CleanupFn>,
    ) -> c_int;
    fn pam_get_data(pamh: *mut PamHandle, module_data_name: *const c_char, data: *mut *const c_void) -> c_int;
}

/// Nombre con el que asociamos la passphrase al handle PAM.
const DATA_KEY: *const c_char = c"pam_tawasuyu_authtok".as_ptr();

/// Cleanup de la passphrase guardada: la sobreescribe y libera.
extern "C" fn free_authtok(_pamh: *mut PamHandle, data: *mut c_void, _err: c_int) {
    if data.is_null() {
        return;
    }
    // SAFETY: `data` es una `CString` que pusimos con `into_raw` en authenticate.
    unsafe {
        let p = data as *mut c_char;
        // Zeroear el contenido antes de liberar (no dejar la frase en el heap).
        let len = libc::strlen(p);
        std::ptr::write_bytes(p as *mut u8, 0, len);
        drop(CString::from_raw(p));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  auth: capturar la passphrase para la fase de sesión
// ─────────────────────────────────────────────────────────────────────────────

/// SAFETY: ABI de PAM. `pamh` es el handle de la transacción; los punteros los
/// gestiona libpam. No tomamos decisión de autenticación.
#[no_mangle]
pub extern "C" fn pam_sm_authenticate(
    pamh: *mut PamHandle,
    _flags: c_int,
    _argc: c_int,
    _argv: *const *const c_char,
) -> c_int {
    let mut item: *const c_void = std::ptr::null();
    // `pam_unix` (antes en el stack) ya puso la password en PAM_AUTHTOK.
    let r = unsafe { pam_get_item(pamh, PAM_AUTHTOK, &mut item) };
    if r == PAM_SUCCESS && !item.is_null() {
        // Copiamos la C-string a una CString propia y la asociamos al handle.
        let copia = unsafe { CStr::from_ptr(item as *const c_char) }.to_owned();
        let raw = copia.into_raw();
        unsafe {
            pam_set_data(pamh, DATA_KEY, raw as *mut c_void, Some(free_authtok));
        }
    }
    PAM_IGNORE
}

/// SAFETY: ABI de PAM. Requerido para módulos de `auth`; no manejamos credenciales.
#[no_mangle]
pub extern "C" fn pam_sm_setcred(
    _pamh: *mut PamHandle,
    _flags: c_int,
    _argc: c_int,
    _argv: *const *const c_char,
) -> c_int {
    PAM_SUCCESS
}

// ─────────────────────────────────────────────────────────────────────────────
//  session: desbloquear la identidad como el usuario
// ─────────────────────────────────────────────────────────────────────────────

/// SAFETY: ABI de PAM. En `open_session` desbloqueamos best-effort.
#[no_mangle]
pub extern "C" fn pam_sm_open_session(
    pamh: *mut PamHandle,
    _flags: c_int,
    argc: c_int,
    argv: *const *const c_char,
) -> c_int {
    // 1) Usuario objetivo.
    let mut user_ptr: *const c_char = std::ptr::null();
    if unsafe { pam_get_user(pamh, &mut user_ptr, std::ptr::null()) } != PAM_SUCCESS || user_ptr.is_null() {
        return PAM_SUCCESS;
    }
    let user = unsafe { CStr::from_ptr(user_ptr) }.to_string_lossy().into_owned();
    // No desbloqueamos identidades de servicio / root.
    if user == "root" || user.is_empty() {
        return PAM_SUCCESS;
    }

    // 2) Passphrase capturada en la fase de auth (puede no haber: login por
    //    clave SSH, autologin… → nada que desbloquear).
    let mut data: *const c_void = std::ptr::null();
    if unsafe { pam_get_data(pamh, DATA_KEY, &mut data) } != PAM_SUCCESS || data.is_null() {
        return PAM_SUCCESS;
    }
    let passphrase = unsafe { CStr::from_ptr(data as *const c_char) }.to_string_lossy().into_owned();

    // 3) uid/gid/home del usuario.
    let Some(u) = lookup_user(&user) else {
        return PAM_SUCCESS;
    };

    // 4) Ejecutar `agora-cli desbloquear` COMO el usuario.
    let agora = parse_agora_cli(&collect_args(argc, argv));
    desbloquear_como_usuario(&u, &user, &passphrase, &agora);
    PAM_SUCCESS
}

/// SAFETY: ABI de PAM. La limpieza del keyring la hace `pam_keyinit revoke`.
#[no_mangle]
pub extern "C" fn pam_sm_close_session(
    _pamh: *mut PamHandle,
    _flags: c_int,
    _argc: c_int,
    _argv: *const *const c_char,
) -> c_int {
    PAM_SUCCESS
}

// ─────────────────────────────────────────────────────────────────────────────
//  Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Datos del usuario que necesitamos para bajar privilegios.
struct UserInfo {
    uid: u32,
    gid: u32,
    home: String,
}

/// Resuelve `name` vía `getpwnam_r`. `None` si no existe.
fn lookup_user(name: &str) -> Option<UserInfo> {
    let cname = CString::new(name).ok()?;
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0i8; 4096];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe {
        libc::getpwnam_r(cname.as_ptr(), &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result)
    };
    if rc != 0 || result.is_null() {
        return None;
    }
    let home = if pwd.pw_dir.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(pwd.pw_dir) }.to_string_lossy().into_owned()
    };
    Some(UserInfo { uid: pwd.pw_uid, gid: pwd.pw_gid, home })
}

/// Lanza `agora-cli desbloquear` como el usuario, con la frase en el entorno.
/// Best-effort: ignora el resultado (no debe afectar el login).
fn desbloquear_como_usuario(u: &UserInfo, user: &str, passphrase: &str, agora_cli: &str) {
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new(agora_cli);
    cmd.arg("desbloquear")
        .uid(u.uid)
        .gid(u.gid)
        .env("HOME", &u.home)
        .env("USER", user)
        .env("LOGNAME", user)
        .env("AGORA_PASSPHRASE", passphrase)
        // Que agora-keystore resuelva ~/.local/share del usuario, no del greeter.
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // Esperamos para que la seed esté cacheada antes de que arranque la sesión.
    let _ = cmd.status();
}

/// Junta los args del módulo (`argv` de PAM) en `Vec<String>`.
fn collect_args(argc: c_int, argv: *const *const c_char) -> Vec<String> {
    let mut out = Vec::new();
    if argv.is_null() || argc <= 0 {
        return out;
    }
    for i in 0..argc as isize {
        let p = unsafe { *argv.offset(i) };
        if !p.is_null() {
            out.push(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned());
        }
    }
    out
}

/// Resuelve la ruta de `agora-cli` desde los args del módulo
/// (`agora_cli=/ruta`), con default a la ubicación del instalador.
fn parse_agora_cli(args: &[String]) -> String {
    for a in args {
        if let Some(v) = a.strip_prefix("agora_cli=") {
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    "/usr/local/bin/agora-cli".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agora_cli_default_y_override() {
        assert_eq!(parse_agora_cli(&[]), "/usr/local/bin/agora-cli");
        assert_eq!(
            parse_agora_cli(&["debug".into(), "agora_cli=/opt/x/agora-cli".into()]),
            "/opt/x/agora-cli"
        );
        // `agora_cli=` vacío cae al default.
        assert_eq!(parse_agora_cli(&["agora_cli=".into()]), "/usr/local/bin/agora-cli");
    }

    #[test]
    fn lookup_root_existe() {
        // root siempre existe; uid 0, home no vacío.
        let r = lookup_user("root").expect("root debe resolver");
        assert_eq!(r.uid, 0);
        assert!(!r.home.is_empty());
    }

    #[test]
    fn lookup_usuario_inexistente_es_none() {
        assert!(lookup_user("usuario-que-no-existe-xyz123").is_none());
    }
}
