//! Prueba interactiva de `brahman-auth` contra PAM. Sirve para verificar
//! la configuración de `/etc/pam.d/<servicio>` en una máquina real.
//!
//! `cargo run -p brahman-auth --example auth-probe -- [usuario] [servicio]`
//!
//! Pide la contraseña sin eco. El servicio por defecto es `mirada`; si
//! `/etc/pam.d/mirada` aún no está instalado, probar con `login`.

use auth_core::{Authenticator, PamAuthenticator};

fn main() {
    let mut args = std::env::args().skip(1);
    let user = args
        .next()
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "root".into());
    let service = args.next().unwrap_or_else(|| "mirada".into());

    let password = match rpassword::prompt_password(format!("Contraseña de {user}: ")) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("no se pudo leer la contraseña: {e}");
            std::process::exit(2);
        }
    };

    let auth = PamAuthenticator::new(&service);
    println!("autenticando «{user}» contra el servicio PAM «{service}»…");
    match auth.authenticate(&user, &password) {
        Ok(info) => {
            println!("✓ autenticado");
            println!("  uid={}  gid={}", info.uid, info.gid);
            println!("  home={}", info.home.display());
            println!("  shell={}", info.shell.display());
        }
        Err(e) => {
            eprintln!("✗ {e}");
            std::process::exit(1);
        }
    }
}
