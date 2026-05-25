# brahman-auth

Autenticación del escritorio. Contrato `Authenticator` agnóstico del
backend, con dos implementaciones.

## Para qué

El greeter de carmen (mirada) necesita verificar la contraseña del
usuario y, en éxito, saber su `uid/gid/home/shell` para arrancar la
sesión. Eso es exactamente lo que entrega `Authenticator::authenticate`:

```rust
use brahman_auth::{Authenticator, PamAuthenticator};

let auth = PamAuthenticator::carmen();
match auth.authenticate("sergio", &password) {
    Ok(info) => arrancar_sesion(info),      // info: UserInfo
    Err(e)   => mostrar_error_en_greeter(e),
}
```

## Backends

- **`PamAuthenticator`** — verifica contra PAM (`/etc/pam.d/<servicio>`),
  el mismo subsistema de `login` y `sudo`. Hereda lo que el
  administrador configure ahí (2FA, FIDO2, `pam_faillock`…) sin que el
  crate lo sepa.
- **`MockAuthenticator`** — credenciales fijas en memoria. Para tests y
  para iterar el greeter en cajas sin PAM configurado.

`AuthError` es deliberadamente grueso: el greeter sólo distingue
"reintentá" (`BadCredentials`) de "cuenta vetada" (`AccountUnavailable`),
y nunca puede saber si un usuario existe.

## Servicio PAM

`data/carmen` es el archivo de servicio. Instalarlo:

```sh
install -Dm644 data/carmen /etc/pam.d/carmen
```

Ajustar el `include` a la pila de login de la distribución (ver los
comentarios del archivo).

## Probar contra PAM en una máquina real

```sh
cargo run -p brahman-auth --example auth-probe -- "$USER" login
```

Pide la contraseña sin eco e informa el `UserInfo` resuelto.
