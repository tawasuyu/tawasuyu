# ssh — cliente SSH mínimo para tawasuyu

Envoltura de `russh` reducida a lo que tawasuyu necesita: **transporte cifrado +
autenticación + un canal de ejecución de comandos**. No pretende cubrir todo
OpenSSH; la shell interactiva real la cubrirá `shuma`
(ver `project_shuma_shell_roadmap`).

## Qué expone

- `Client` — conecta, autentica y ejecuta comandos remotos (async, sobre tokio).
- `Config` — parámetros de conexión.
- `Auth` — método de autenticación (clave / contraseña).

## No-objetivos (hoy)

- No es un reemplazo de OpenSSH ni de mosh/tmux.
- No hace multiplexación de sesiones ni reconexión.

## Estado (2026-05-31)

### Hecho
- `Client`: conexión, autenticación (clave/contraseña) y exec de comandos.
- API async sobre tokio + tipos de configuración/error.

### Pendiente
- `Server` (aceptar conexiones + handler de exec) — sólo mencionado, no implementado.
- PTY interactivo, port-forwarding y SFTP.
- Multiplexación / reconexión (deferido a `shuma`).
- Tests de integración (hoy sin cobertura automatizada).

## Lugar en el repo

`shared/ssh` — cliente SSH mínimo. La experiencia de shell remota la integra
`shuma`.
