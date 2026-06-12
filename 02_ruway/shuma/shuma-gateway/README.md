# shuma-gateway

> Remote-session gateway of [shuma](../README.md).

Adaptador **HTTP/JSON + WebSocket** sobre el admin socket del `shuma-daemon`.
Pensado para clientes no-Rust (app Android, web, curl) que no hablan postcard.

```
cliente JSON/WS  ──►  shuma-gateway  ──postcard──►  shuma-daemon (unix socket)
```

## Deps

- [`shuma-protocol`](../sandbox/shuma-protocol/README.md)
- `axum` (incluye WebSocket)

## Ejecutar

```sh
cargo run -p shuma-gateway
```

| Var de entorno | Default | Para qué |
|-----|---------|----------|
| `SHIPOTE_GATEWAY_LISTEN` | `127.0.0.1:7378` | dirección TCP de escucha |
| `SHIPOTE_GATEWAY_TOKEN`  | *(vacío)* | si se define, exige auth (ver abajo) |
| `SHIPOTE_GATEWAY_LOG`    | `info` | filtro de `tracing` |

El daemon debe estar corriendo; el gateway se conecta a su socket
(`$XDG_RUNTIME_DIR/shuma.sock`).

## Auth (opcional)

Sin `SHIPOTE_GATEWAY_TOKEN` el gateway queda **abierto** (úsalo en loopback o
detrás de un túnel TLS/SSH/Noise). Con token definido, toda request exige:

- header `Authorization: Bearer <token>`, o
- query `?token=<token>` (para clientes WS que no fijan headers).

Comparación en tiempo constante. Sin auth → `401`.

## `POST /rpc` — request/response 1:1

Body = un `shuma_protocol::Request` como JSON; respuesta = el `Response` como
JSON. Los enums van **externally-tagged** (convención serde):

- variante unitaria → string: `"Ping"`, `"Health"`, `"WorkspaceList"`, `"Capabilities"`.
- variante con campos → objeto de una clave: `{"WorkspaceStop":{"id":…,"grace_ms":1000}}`.

Ejemplos verificados:

```sh
curl -s --noproxy '*' -XPOST localhost:7378/rpc -d '"Ping"'
# "Pong"

curl -s --noproxy '*' -XPOST localhost:7378/rpc -d '"Health"'
# {"Health":{"version":"0.1.0","uptime_ms":667,"alive_workspaces":0,
#   "alive_commands":0,"alive_pipelines":0,"active_flows":0,"dirty":false}}

curl -s --noproxy '*' -XPOST localhost:7378/rpc -d '"WorkspaceList"'
# {"WorkspaceList":{"items":[{"id":…,"label":"…","commands":0,"uptime_ms":…}]}}
```

Requests útiles para un panel de control (ver `shuma-protocol::Request` para el
conjunto completo y los campos exactos):

| Request (JSON) | Para qué |
|----------------|----------|
| `"Health"` | versión + uptime + conteos vivos |
| `"WorkspaceList"` | listar workspaces (= "claudes") |
| `{"WorkspaceCreate":{"spec":{…WorkspaceSpec…}}}` | crear workspace |
| `{"WorkspaceStop":{"id":…,"grace_ms":1000}}` | detener (SIGTERM→SIGKILL) |
| `{"WorkspaceStats":{"workspace":…}}` | CPU/RSS/comandos vivos |
| `{"WorkspaceFullSummary":{"workspace":…}}` | stats+quota+commands en 1 roundtrip |

Errores: `400` (JSON inválido), `502` (`{"error":"daemon: …"}`), `401` (auth).

## `GET /ws/pty` — terminal remoto (WebSocket ↔ subprotocolo `ExecPty`)

Canal **full-duplex** hacia un PTY remoto. Ideal para "un terminal por cada
Claude" (`program:"claude"`), un `ssh host`, o cualquier TUI.

1. **Abrir** — primer mensaje del cliente = **texto JSON** con el spec:
   ```json
   {"cwd":"/ruta","program":"claude","args":["code"],"rows":40,"cols":120}
   ```
   (`cwd` default `"."`, `rows` 24, `cols` 80, `args` []).
2. **Salida** — el server manda **frames binarios** = bytes crudos del PTY
   (con escapes ANSI). Aliméntalos a un emulador vt100.
3. **Teclas** — el cliente manda **frames binarios** = stdin crudo.
4. **Resize** — el cliente manda **texto JSON** `{"t":"resize","rows":50,"cols":100}`.
5. **Fin** — al salir el proceso, el server manda **texto JSON**
   `{"t":"exit","code":0}` (o `{"t":"error","msg":"…"}`) y cierra el WS.
6. **Abortar** — el cliente **cierra el WS** → el daemon mata el PTY (SSH/PTY).

Regla: **binario = bytes del terminal** (ambos sentidos); **texto = control JSON**.

### ⚠️ Persistencia (clave para "administrar un grupo de claudes")

Hoy un `ExecPty` es **efímero**: el proceso vive sólo mientras el WebSocket está
abierto; al cerrar (cerrar la app, caída de red) **el proceso muere**. Sirve
para asomarse a una sesión, no para dejar claudes corriendo y re-adjuntarse
luego. Para sesiones **persistentes** tipo tmux (dejar N claudes trabajando y
attach/detach desde el móvil) hace falta un modo de PTY persistente en el daemon
— pendiente de decidir.

## Cliente Android (rizoma `:consola`, planeado)

- Lista de claudes: `POST /rpc "WorkspaceList"` (+ `WorkspaceStats` por item).
- Terminal: WebSocket a `/ws/pty` con `program:"claude"`, emulador vt100, Trazo
  como teclado.
- Auth: token en Android Keystore → header `Authorization: Bearer`.
