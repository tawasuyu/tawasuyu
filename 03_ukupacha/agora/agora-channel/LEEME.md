# agora-channel

> Puente entre las identidades Ed25519 de [agora](../LEEME.md) y el contrato del canal de release wawa en [`format`](../../../shared/format).

`format` declara los tipos del cable — `Canal`, `RaizFirmada`, `mensaje_a_firmar` — pero dice explícito que *"la verificación vive en `agora` (o en `firma`)"*. Este crate es esa verificación. Usa [`agora-core::Keypair`](../agora-core/LEEME.md) para producir y comprobar firmas sobre el mensaje canónico que `format::mensaje_a_firmar(nombre_canal, timestamp, raiz)` define.

## Qué hace

- `firmar_raiz(kp, canal_nombre, raiz, timestamp) -> RaizFirmada` — firma una raíz de manifiesto para un canal y produce la entrada del cable.
- `verificar_raiz(autor, canal_nombre, raiz)` — re-verifica una `RaizFirmada` contra la clave pública del autor del canal. Detecta firmas forjadas, truncadas o replayed.
- `verificar_canal(canal)` — recorre todo el historial `raices` de un `Canal`, verificando cada entrada bajo el `autor` del canal y exigiendo monotonicidad estricta de `timestamp` (sin replays de pasado-después-de-futuro).
- `firmar_para_anuncio(kp, canal_nombre, raiz, timestamp) -> (AgoraId, Firma)` — produce sólo el par `(autor, firma)` que va en `MensajeAkasha::AnunciarCanal`, para que un caller que *sí* depende del crate `akasha` pueda ensamblar el frame sin que este crate lo necesite.

## Lo que deliberadamente *no* hace

- **No** depende de `akasha`. `MensajeAkasha::AnunciarCanal` vive en el stack bare-metal de wawa y se excluye del workspace global. `agora-channel` produce las piezas criptográficas; el ensamble del frame le corresponde al lado que sí tiene los tipos de red.
- **No** decide política de confianza. Si el autor del canal es de fiar primero es una pregunta del grafo de confianza local — `agora-channel` sólo asevera hechos criptográficos ("esta firma es válida").

## Cierra

- `PLAN.md:177` (*"Identidad agora Ed25519 firmable — pendiente"*).
- `WAWA.md §14.1` (*"verificación de firma + re-anclaje quedan para userspace"*) — el lado userspace de la verificación de firma ya existe en Rust puro.

## Deps

- [`agora-core`](../agora-core/LEEME.md) (por `Keypair` y `verify_signature`)
- [`format`](../../../shared/format/) (por `Canal`, `RaizFirmada`, `mensaje_a_firmar`, `AgoraId`, `Firma`, `Hash`)
- `thiserror`
