# tullpu

**Un editor de imágenes por capas donde nada se destruye nunca.**

*Read this in English: [README.md](README.md).*

tullpu es el editor de imágenes de tawasuyu. Trabajás en capas, como en
cualquier editor serio — pero acá la pila de capas es un **DAG direccionado
por contenido**: cada capa es un nodo, cada ajuste, filtro u operación IA
es una *capa derivada* que apunta a su madre. Cambiás la madre y el cono
derivado queda *stale*; la regeneración es bajo demanda. Nada pisa los
píxeles que ya tenías.

Sigue las dos reglas de la casa: el modelo es **agnóstico de UI** (sin
tipos de Llimphi ni de modelos IA en el core), y las operaciones IA hablan
con un **daemon separado por socket Unix**, así la app nunca linkea un
modelo.

## Arquitectura: cinco pisos

| Crate | Rol |
|---|---|
| `tullpu-core` | El modelo agnóstico: `Capa` (id, hash BLAKE3 de contenido, modo de fusión, opacidad, máscara, origen Raster/Derivada), `Lienzo`, `GrafoDeCapas`, 28 modos de fusión Photoshop-completos. Serializado vía `format::Objeto` (postcard + dedup BLAKE3). |
| `tullpu-render` | Compositor CPU: recorre el DAG de arriba hacia abajo, funde sobre un buffer `Rgba8`, output `image::RgbaImage`. (GPU compute es upgrade planificado.) |
| `tullpu-paint` | Kernel de pintura ciego a la GUI: pincel, disco, líneas, degradés, flood fill, simetría, src-over, rotaciones 90° — pura matemática de buffers. |
| `tullpu-ops` | El catálogo de operaciones: ops locales (brillo, contraste, niveles, blur, saturación, tonalidad, curvas tonales, máscaras editables, pincel pro) + el orquestador `regenerar_stale_con_ia` que re-deriva los conos stale. |
| `tullpu-app-llimphi` | La app de escritorio: lienzo central, panel de capas, paleta de ops. Binario `tullpu`. |

La plomería IA es la familia **pixel-verbo**, hermana en píxeles del patrón
del daemon de embeddings: `pixel-verbo-core` (trait `Proveedor` agnóstico
de modelo: segmentar / inpaint / restyle / generar), `pixel-verbo-mock`
(proveedor determinista para dev/CI — misma op+prompt, mismo output),
`pixel-verbo-daemon` + `-bin` (un modelo en RAM sirviendo a N procesos
cliente por `$XDG_RUNTIME_DIR/pixel-verbo.sock`).

## Probalo

```bash
cargo run -p tullpu-app-llimphi --release      # el editor (proveedor Mock por defecto)

# opcional: correr el daemon de píxeles; la app lo detecta al arrancar
cargo run -p pixel-verbo-daemon-bin -- --provider mock

cargo test -p tullpu-core -p tullpu-render -p tullpu-ops   # los pisos de lógica
```

Los archivos PSD se importan por `shared/foreign-psd` (el puente de
formatos ajenos de la suite): capas dedupeadas por BLAKE3, modos de fusión
mapeados.

## Estado

MVP+: core + render + app funcionando; catálogo completo de 28 modos de
fusión; ops locales e IA; curvas tonales, máscaras editables, pincel pro;
import PSD. Pendiente: nodegraph visual sobre el DAG de capas, proveedor
ONNX real (hoy Mock), tiling para imágenes enormes, compositing GPU,
export PSD.
