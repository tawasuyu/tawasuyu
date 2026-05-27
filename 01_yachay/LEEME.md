# 01 yachay · conocer

`yachay` (quechua: *saber, conocimiento*). Es el cuadrante del **modelo**: lo que tomamos de la percepción y lo organizamos como teoría. Acá viven los simuladores, los catálogos, los grafos semánticos, las hojas de cálculo, los motores físicos. Lo que `unanchay` recibió en bruto, `yachay` lo convierte en estructura que se puede consultar y proyectar.

La regla del cuadrante es **el modelo se valida contra la realidad, no contra sí mismo**: una teoría elegante que no se chequea es decoración. Cada aplicación de `yachay` tiene una manera explícita de confrontar su salida con datos externos.

## Aplicaciones

- **[cosmos](cosmos/README.md)** — astronomía con precisión astronómica. Tiempo, efemérides, coordenadas, WCS, astrología, validación contra ephemerides oficiales.
- **[dominium](dominium/README.md)** — simulador determinista de campo medio: cinco capas físicas (materia · psique · poder · oro · degradación) + agentes vectoriales + acoplamiento ψ↔acción endógeno.
- **[iniy](iniy/README.md)** — laboratorio semántico. Subjective Logic + dirección de subjetividad para auditar afirmaciones. Piloto: auditoría de libros y wikis.
- **[nakui](nakui/README.md)** — motor reactivo tipo Excel sobre principios sólidos: Decimal exacto, cascada topológica, WAL, time-travel, invariantes atómicos. Tres vistas (matriz · grafo · formulario) sobre el mismo grafo de tokens.
- **[tinkuy](tinkuy/README.md)** — motor de partículas DOD (ECS-SoA + Grid3D + Velocity-Verlet paralelo) con snapshots BLAKE3 compatibles con Wawa.

## Manifiesto

> **Conocer es atreverse a equivocarse con precisión.**
> El modelo no se vende como verdad; se ofrece como herramienta. Su valor está en que falla de manera predecible.
>
> 1. **Determinismo siempre que sea posible.** Misma semilla, mismo resultado — la reproducibilidad es honestidad.
> 2. **Exactitud sobre estética.** `Decimal` y enteros antes que `f32` cuando el dominio lo pide (nakui, cosmos).
> 3. **Las unidades no son adorno.** SI, IAU, ISO — siempre explícitas; nunca "asumir grados".
> 4. **Validar contra ephemerides, datos, simulaciones independientes.** Si nadie más obtiene tu resultado, sospechá del tuyo primero.
