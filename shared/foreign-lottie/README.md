# foreign-lottie

Fork vendorizado de **[velato](https://github.com/linebender/velato) 0.9.0**
(reproductor de Lottie de Linebender, `Apache-2.0 OR MIT`) — el motor que parsea
un `.json` de Lottie y lo emite a una `vello::Scene`. Entra al repo como puente
de formato ajeno (Regla 4 de `CLAUDE.md`: los formatos ajenos viven en
`shared/foreign-*`, nunca en el núcleo de las apps).

Lo consume `02_ruway/llimphi/llimphi-lottie` (el puente fino a Llimphi).

## Por qué un fork y no la dependencia de crates.io

velato 0.9 es la única versión que clava `vello 0.7` (la del workspace), así que
arrancó como dependencia directa. Pero su **importador paniquea**
(`todo!()`/`unimplemented!()`) ante features perfectamente válidas de Lottie —
y eso corre en el hilo de UI. Forkeamos para que **degrade con gracia** en vez
de tumbar la app.

## Cambios respecto de velato 0.9.0 upstream

Todos marcados con `// FORK tawasuyu:` en el código. Concentrados en
`src/import/converters.rs`:

| Caso upstream (panic) | Fork (degradación graciosa) |
|---|---|
| transform sin campo de rotación → `todo!("split rotation")` | rotación 0 |
| `SplitRotation` (rot. x/y/z separadas) → `todo!()` | usa la componente z (la 2D visible) |
| `SplitPosition` en shape-transform → `todo!("split position")` | arma la posición desde x/y |
| asset desconocido (imagen embebida…) → `unimplemented!()` | se omite el asset |
| blend `Add` → `unimplemented!()` | compositing aditivo (`Compose::Plus`) |
| blend `HardMix` → `unimplemented!()` | `HardLight` (el mix más cercano) |

El camino de render (`runtime/`) quedó intacto — es panic-free. Los `panic!` que
restan en `src/schema/*` son test-only o del lado serializador (que no usamos);
`llimphi-lottie` igual envuelve el parse en `catch_unwind` como red secundaria.

## Pendiente (gaps del propio velato, no introducidos por el fork)

velato no implementa todavía, y acá tampoco (degradan a omisión, no a panic):
**capas de texto**, **effects** (blur/drop-shadow) y **expresiones**. Declarado
en `src/schema/mod.rs`. Son lo grande; se completan cuando un `.json` concreto
los pida.

## Tests

`tests/no_panic_fork.rs` certifica que cada caso que antes paniqueaba ahora
importa con `Ok`, y que el camino feliz sigue intacto.
