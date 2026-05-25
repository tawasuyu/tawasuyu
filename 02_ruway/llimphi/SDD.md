# Llimphi — motor gráfico soberano

> Llimphi (quechua: *color / brillo / pigmento*, en el sentido de "pintar la pantalla"). Tipo: **NATIVE GPU rendering suite**.

## Tesis

Soberanía total sobre el píxel. Renderizar las geometrías exactas del simulador cósmico (`cosmos`), el compositor (`mirada`), las apps de escritorio (`nahual`) y el visor (`pluma`) sin cajas negras de Apple/Google/navegadores. Reemplazo total de **GPUI** en la pila gioser.

## Anatomía — 4 capas estrictas (S₀ → S₂)

Cada capa hace **una sola cosa** con precisión matemática.

```
[ CUADRANTE III · 0x02 RUWAY ]

4. llimphi-ui      — Lógica de Interfaz (Árbol Monádico / DAG UI)
   │                 (manejo de estado, eventos de teclado/ratón)
   ▼
3. llimphi-layout  — Motor de Layout (Cálculo Espacial)
   │                 (cajas, dimensiones, restricciones flex/grid)
   ▼
2. llimphi-raster  — Rasterizador Vectorial (La Brocha Fina)
   │                 (primitivas matemáticas → píxeles via Compute Shaders)
   ▼
1. llimphi-hal     — Abstracción de Hardware (Puente al Silicio)
   │                 (GPU o Framebuffer, sin importar el OS)
   ▼
[ HARDWARE · GPU / Pantalla ]
```

## Fases de forja

### Fase 1 — Puente al Silicio (`llimphi-hal`)

Aislar el motor del sistema operativo. Llimphi debe pintar tanto en una ventana Wayland controlada por `mirada` como en el framebuffer directo al arrancar `wawa`.

- **Abstractor:** `wgpu` (impl Rust de WebGPU sobre Vulkan nativo). Control de memoria seguro, bajísima sobrecarga.
- **Ventana:** `winit` para desarrollo en Linux. La arquitectura define un **trait `Surface`** abstracto: el día de mañana se desenchufa `winit` y se le pasa el puntero de memoria bruto del kernel `wawa`.
- **Hito:** Compilar, iniciar Vulkan por debajo, limpiar la pantalla pintándola de un solo color gris plomo a 144 Hz.

### Fase 2 — Brocha Matemática (`llimphi-raster`)

Pintar curvas y grafos orbitales con precisión Δ < 10⁻⁹ rad sin destrozar la CPU. En lugar de rasterizar píxel por píxel, **delegar todo el cálculo vectorial a los Compute Shaders de la GPU**.

- **Motor:** `vello`.
- **Integración:** Conectar la textura de salida de `wgpu` como lienzo destino de `vello`.
- **Ejecución:** Construir una `Scene` en `vello`. Pasarle primitivas geométricas puras (líneas, curvas de Bézier, texto).
- **Hito:** Renderizar en pantalla el grafo de un nodo estático con anti-aliasing perfecto calculado íntegramente por la GPU.

### Fase 3 — Física del Espacio (`llimphi-layout`)

Posicionar dinámicamente paneles, texto y ventanas requiere resolver ecuaciones de restricciones espaciales. No escribir un sistema propio de márgenes/padding: es un sumidero infinito.

- **Motor:** `taffy` (de la gente de Dioxus). Algoritmos Flexbox + CSS Grid en Rust puro.
- **Flujo:** Antes de decirle a `llimphi-raster` dónde pintar, pasar el árbol de nodos a `taffy` para calcular las coordenadas `(x, y, width, height)` absolutas de toda la interfaz.
- **Hito:** Paneles laterales y cajas que se redimensionan automáticamente, calculados en < 1 ms por frame.

### Fase 4 — Árbol de Estado Monádico (`llimphi-ui`)

El mayor problema de las interfaces (y por qué falló el paradigma OOP en esto) es el manejo del estado. Aquí se inyecta la cosmovisión estructural.

- **Arquitectura:** Nada de mutabilidad compartida (`Rc<RefCell<...>>` disperso). Unidireccional estilo Elm o **DAG (Grafo Acíclico Dirigido)**: el estado de la aplicación es **inmutable** y cada evento (click, tecla) genera una **nueva versión** del estado.
- **Bucle:**
  1. El usuario hace click (Input).
  2. El evento actualiza el Estado Global.
  3. El Estado Global reconstruye el Árbol UI.
  4. El Árbol pasa por `llimphi-layout` (Layout).
  5. Las coordenadas resultantes generan primitivas para `llimphi-raster` (Scene).
  6. `llimphi-hal` renderiza y hace el swap de la pantalla.

## Veredicto arquitectónico

No es una biblioteca genérica. Es un **motor de combate**. `wgpu + vello + taffy + DAG monádico` da un frontend capaz de competir en rendimiento con los mejores editores del mundo, diseñado como **traje a medida** para las topologías de gioser. Sin abstracciones de navegadores, sin cajas negras de Apple/Google.

## Pila exacta (sin negociación)

| Capa | Crate raíz | Deps externas |
|---|---|---|
| HAL | `llimphi-hal` | `wgpu`, `winit`, `raw-window-handle` |
| Raster | `llimphi-raster` | `vello`, `vello_encoding`, `peniko` |
| Layout | `llimphi-layout` | `taffy` |
| UI | `llimphi-ui` | (puro Rust, sin deps gráficas) |

## Migración GPUI → Llimphi

Apps actualmente en GPUI que deben portarse:

- `02_ruway/nahual/*` (todas las apps GPUI: shell, file-explorer, database-explorer, image-viewer, text-viewer + 8 libs + 12 widgets)
- `02_ruway/mirada/mirada-launcher`, `mirada-portal`, `mirada-greeter`
- `00_unanchay/pluma/pluma-editor-gpui`
- `01_yachay/dominium/dominium-canvas-gpui`
- `01_yachay/cosmos/cosmos-app` (canvas + panels GPUI)

**Estrategia:** Las apps mantienen su lógica de dominio en sus `*-core` agnósticos. Solo se reemplaza la capa de presentación: en lugar de `use gpui::*`, pasan a usar `use llimphi_ui::*`.

## Estado

- **2026-05-25:** SDD escrito. Esqueletos de los 4 crates creados.
- **2026-05-25 (tarde):** Las 4 fases en código y compilando. Examples:
  - `cargo run -p llimphi-hal --example clear_screen --release` — ventana gris plomo a refresh del display ✅ (verificado en hardware).
  - `cargo run -p llimphi-raster --example render_node --release` — nodo con AA perfecto vía vello/wgpu.
  - `cargo run -p llimphi-layout --example layout_panels --release` — sidebar + header/body/footer flex que se reorganiza al resize.
  - `cargo run -p llimphi-ui --example counter --release` — bucle Elm completo: click hit-test → update → view → layout → raster → present.
- **Próximo:** texto (skrifa/parley sobre vello) — necesario para Cosmos, Pluma, Nahual. Luego: migración de las apps GPUI a Llimphi.
