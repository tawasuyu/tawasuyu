# foreign-psd — puente de Photoshop (.psd/.psb)

Puente de **Photoshop** al modelo de capas nativo de `tullpu`. Parsea capas,
grupos, máscaras y los blend modes de Photoshop, y rasteriza al grafo nativo.
Entra por `shared/foreign-*` (regla #4): nunca toca el núcleo de `tullpu`.

## Qué expone

- Parseo de `.psd`/`.psb`: capas, grupos (anidados), máscaras.
- Catálogo completo de blend modes de Photoshop (incluye HSL, comparativos por
  luminosidad y Dissolve) aplicados por-canal.
- Rasterización de grupos (Normal y non-Normal) con propagación al modelo nativo.

## No-objetivos

- Sólo lectura: no escribe `.psd`.
- No es editor; alimenta a `tullpu`, que trabaja en formato nativo.

## Estado (2026-05-31)

### Hecho
- Parseo de capas/grupos/máscaras + catálogo Photoshop de blend modes completo
  (12 por-canal + HSL + comparativos + Dissolve).
- Aplanado de grupos con propagación; rasterización de grupos non-Normal
  (hasta fase 15).
- Tests por blend mode/grupo (≈14).

### Pendiente
- Efectos de capa (sombras, bisel, etc.) y ajustes (curves/levels).
- Texto vivo y smart objects (hoy rasterizados, no editables).
- Perfiles de color / espacios distintos a RGB8.

## Lugar en el repo

`shared/foreign-psd` — puente de formato Photoshop. Consumidor: `tullpu`.
