# agora-app

> UI Llimphi de [agora](../LEEME.md): identidades, atestaciones, compositor, política.

Cuatro tiles draggables sobre el mismo `TrustGraph`, intercambiables por arrastre de la barra de título:

- **Identidades** — conocidas + mías. El botón nueva-identidad genera una seed CSPRNG, la encierra en [`agora-keystore`](../agora-keystore/LEEME.md) y registra la cara pública en el grafo.
- **Atestaciones** — la pila de evidencia verificada. Filtrable por sujeto / atestador / predicado. Auto-atestaciones marcadas distinto.
- **Compositor** — edición in-situ de `sujeto · predicado = valor`, firmado como la identidad local activa, agregado al grafo y persistido al confirmar.
- **Política** — slider de `min_third_party`, checkbox de `accept_self`, veredicto en vivo sobre el claim seleccionado.

La persistencia es automática: cada cambio pasa por [`agora-store`](../agora-store/LEEME.md); las claves privadas viven en `agora-keystore`, desbloqueadas al arrancar con una passphrase.

## Uso

```sh
cargo run --release -p agora-app
```

## Deps

- Todos los `agora-*` (core, graph, store, keystore)
- [`llimphi-ui`](../../../02_ruway/llimphi/llimphi-ui/) + widgets de Llimphi
