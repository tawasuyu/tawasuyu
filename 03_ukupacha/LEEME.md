# 03 ukupacha · raíz

`ukupacha` (quechua: *mundo interior, raíz, lo subterráneo*). Es el cuadrante de la **infraestructura invisible**: el kernel, el bootloader, el filesystem, los protocolos de red profundos, la comunidad que sostiene todo. Lo que ningún usuario ve directamente pero que decide si el sistema arranca o no.

La regla del cuadrante es **la invariante antes que la feature**: en `ukupacha` los breaking changes cuestan migraciones a todo el árbol; por eso cada decisión se piensa como "¿en diez años, esto sigue siendo verdad?". El cambio acá es lento y deliberado.

## Aplicaciones

- **[agora](agora/README.md)** — plaza pública. Foro, conversación, deliberación con identidad mínima.
- **[arje](arje/README.md)** — bootloader y vida temprana del sistema. `arje-seeds` (semillas), `arje-packager` (empaquetado), `arje-installer` (instalación), `arje-absorb` (ingestión de un sistema existente).
- **[minga](minga/README.md)** — colaboración entre nodos. Tradición andina del trabajo comunitario, aplicada a la red.
- **[wawa](wawa/README.md)** — sistema operativo desde cero (`wawa-kernel`, `wawa-boot`, `wawa-fs`, `apps/`). Ingesta POSIX → BLAKE3; el filesystem como DAG content-addressed; gaming-grade (AOT WASM + GPU passthrough + frame pacing cooperativo).
- **[wawa-explorer](wawa-explorer/README.md)** — visor host-side del DAG de Wawa: lee `.img`, habla el protocolo Akasha por raw sockets, muestra el árbol con detalle en Llimphi.

## Manifiesto

> **La raíz se sostiene callada.**
> Lo que dura es lo que no llama la atención cuando funciona. El kernel buen kernel es el que nadie nota.
>
> 1. **Sin dependencias frívolas en la raíz.** Cada crate de `ukupacha` justifica cada `Cargo.toml` línea por línea.
> 2. **Content-addressed por defecto.** BLAKE3 es la identidad — los bytes son la verdad, los nombres son hint.
> 3. **El usuario no es el cliente del kernel.** El cliente del kernel es el operador. Las herramientas amigables viven en `02_ruway`.
> 4. **Documentar como si el próximo lector fuera un arqueólogo dentro de veinte años.** Los SDDs, los WHY, los porqués escritos — son la única forma de que algo sobreviva al autor.
