# mirada-greeter

El greeter (pantalla de login) del escritorio carmen.

Una ventana GPUI: el compositor `mirada-compositor`, cuando bootea en
modo greeter, la arranca como proceso hijo, la compone a pantalla
completa (la reconoce por `app_id = "carmen.greeter"`) y le lee el
stdout.

## Flujo

1. El usuario teclea usuario + contraseña. `Enter` en «usuario» pasa el
   foco a «contraseña»; `Enter` en «contraseña» autentica.
2. La autenticación corre con [`brahman-auth`] en un hilo de fondo (PAM
   puede demorar ~2 s ante un fallo, no se congela la UI).
3. En éxito, el greeter **imprime un `SessionTicket` a stdout** y
   termina. El compositor parsea esa línea y hace el traspaso a modo
   sesión sin reiniciar el servidor gráfico.

La línea de tiquet lleva el prefijo `MIRADA-SESSION-TICKET-v1`; el resto
del stdout (logs) se ignora.

## Backend de autenticación

| Entorno | Backend |
|---|---|
| (por defecto) | PAM, servicio `carmen` (`/etc/pam.d/carmen`) |
| `MIRADA_GREETER_PAM=<servicio>` | PAM con otro servicio |
| `MIRADA_GREETER_MOCK=usuario:secreto` | Mock — credenciales fijas |

El modo mock sirve para iterar la UI en cajas sin PAM o con el greeter
anidado dentro de otro escritorio:

```sh
MIRADA_GREETER_MOCK=demo:demo cargo run -p mirada-greeter
```

## Integración con el compositor

El consumo del tiquet ya está cableado. `mirada-compositor --greeter`
lanza este greeter, lee su stdout y, al recibir el `SessionTicket`,
muta de `BodyMode::Greeter` a `BodyMode::Session` y arranca la sesión
del usuario con `setuid`/`setgid` — sin reiniciar el servidor Wayland.
Ver el README de `mirada-compositor`, sección **Modo greeter (DM)**.
