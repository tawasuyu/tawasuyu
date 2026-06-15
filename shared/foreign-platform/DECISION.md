# foreign-platform — la decisión de diseño

> ¿Es ineficiente montar YouTube (y demás) en una capa donde todo lo demás es
> agnóstico? ¿Conviene incluso describir los proveedores como *scripts de
> texto* para sumar otros? — Sí a la capa agnóstica; sí a los scripts **para la
> familia REST limpia**; **no** para YouTube directo.

## Dos cosas que se confunden

### 1. El trait agnóstico — no se discute
`PlatformProvider` (`search / trending / channel_videos / video`) con tipos de
dominio comunes (`VideoCard`, `StreamSet`, `VideoDetail`). El frontend habla
sólo con esto y no sabe quién hay detrás (regla #2 del repo). Costo en runtime:
un `dyn` dispatch por llamada, despreciable frente a la latencia de red. **No es
ineficiente.**

### 2. ¿Providers en Rust o en texto?
Acá está la intuición buena — pero vale para **una mitad** del universo:

- **Familia REST limpia** (Invidious, Piped, **PeerTube**): son "pegá a este
  endpoint, mapeá estos campos JSON a mi tipo". Eso **se datifica**: un motor
  genérico (`rest::RestProvider`) + un descriptor de texto (`descriptors/*.ron`)
  por API. **Sumar un proveedor o instancia compatible = un archivo `.ron`, cero
  Rust.** Es lo que `FREETUBE.md` ya quiere priorizar (lo federado e Invidious
  por sobre YouTube directo). Aquí tu idea es oro.

- **YouTube directo (Innertube)**: *resiste* la datificación. No es REST: tiene
  descifrado de firma, el transform del parámetro `n` (un challenge JS que
  **cambia solo**), tokens de continuación tipo protobuf, máquinas de estado de
  paginación. Por eso `youtubei.js` y `yt-dlp` son bases imperativas en
  mantenimiento perpetuo. Un descriptor no resuelve el cipher-JS.

## La salida

YouTube *directo* casi nunca hace falta como bridge propio: se enruta por una
instancia **Invidious** (que ya es REST → descriptor) o por **`foreign-ytdlp`**
(que ya existe y resuelve el stream). O sea: la única pieza que resiste el
enfoque data-driven es también la que el repo menos quiere — y para la que ya
hay atajo. Nunca entra a este crate.

## Qué quedó construido (primera piedra)

```
shared/foreign-platform/
  src/
    model.rs         tipos de dominio agnósticos
    provider.rs      trait PlatformProvider + PlatformError
    json.rs          extractor de paths con puntos sobre serde_json::Value
    rest.rs          motor data-driven: RestDescriptor + RestProvider<HttpFetch>
    descriptors/
      invidious.ron  API /api/v1 de Invidious (array en raíz)
      peertube.ron   API /api/v1 de PeerTube (lista anidada en "data")
    lib.rs           re-exports + registro de descriptores + tests sobre fixtures
```

Los tests validan el camino descriptor→mapeo **sin red** (fixtures), como el
resto de los núcleos agnósticos del repo. Dos proveedores heterogéneos (raíz
array vs lista anidada, `videoId` vs `uuid`) mapean al mismo trait con sólo
texto distinto: esa es la prueba de la apuesta.

## Pendiente (siguiente capa, no en esta piedra)

- `comments` / `trending` por categoría / paginación con cursor (algunas APIs
  no son `page` simple).
- Selección de "mejor formato" en `map_stream` (hoy MVP: primer muxeado, o
  primer video+audio). Falta ordenar por resolución/bitrate.
- Frontend Llimphi (el "FreeTube"): grilla de resultados, página de canal,
  suscripciones locales (almacenamiento direccionado por contenido), que
  **componga** `foreign-platform` (descubrir) + `media` (reproducir).
- Integraciones de comunidad (SponsorBlock como segmentos sobre el timeline de
  `media`), si se decide federarlas sobre `agora`/`minga`.
- `descriptors/piped.ron` y demás: ahora es sólo agregar texto.
