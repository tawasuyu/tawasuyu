# Llimphi · Android

Port nativo de Llimphi a Android. Una `NativeActivity` en C que
delega al `android_main` que `android-activity` exporta desde la
`.so` Rust, idéntico patrón que un binario `main()` en desktop.

## Estado

| crate | estado |
|---|---|
| `clear-screen-android` | ✓ APK firmado v2, instalable en Android 7+ |
| resto de apps Llimphi | pendientes — el patrón es reusar `android_main` |

## Tesis

El motor Llimphi (HAL + raster + layout + text + ui) **no se toca**.
Lo único nuevo por target Android es:

1. Entry-point `#[no_mangle] android_main(app: AndroidApp)` en vez de
   `fn main()`.
2. Construir el `EventLoop` con `with_android_app(app)` para que
   `winit` reciba `Resumed` / `Suspended` / `InputAvailable` desde el
   Looper de Android.
3. Recrear la `Surface` en cada `Resumed`: Android invalida la
   NativeWindow al pasar a background. El `App::state: Option<State>`
   ya está estructurado para eso.

Las apps existentes que viven sobre Llimphi compilan sin cambios — lo
que se reescribe es el **lifecycle wrapper**, no la lógica de render
ni los widgets.

## Cómo construir

Una sola pasada — el script wrapper:

```sh
./scripts/build-android.sh clear-screen-android
```

Resultado: `target/x/release/android/clear-screen-android.apk`
firmado con APK Signature Scheme v2, listo para
`adb install -r <apk>`.

## Setup inicial (una vez por máquina)

```sh
# Targets Rust
rustup target add aarch64-linux-android x86_64-linux-android

# Wrapper de build de Rust mobile (binario `x`)
cargo install xbuild

# NDK r27c (~640 MB descomprimido, ~1.5 GB)
curl -L -o /tmp/ndk.zip \
  https://dl.google.com/android/repository/android-ndk-r27c-linux.zip
unzip /tmp/ndk.zip -d $HOME/
export ANDROID_NDK_HOME=$HOME/android-ndk-r27c

# SDK (sólo build-tools + platform-tools, no se necesita la plataforma
# completa porque el APK se genera con aapt2 + apksigner del SDK).
# En Artix viene del paquete `android-sdk-build-tools`.
```

El script `build-android.sh` genera automáticamente un PEM RSA2048
self-signed en `~/.local/share/llimphi-android/debug.pem` la primera
vez que corre. Para firma de release usar un PEM propio y exportarlo
en `LLIMPHI_PEM`.

## Estructura del APK generado

```
clear-screen-android.apk
├── AndroidManifest.xml          ← xbuild genera; NativeActivity
└── lib/arm64-v8a/
    └── libclear_screen_android.so  ← 7.5 MB sin strip, ~2 MB stripped
```

Sin assets, sin recursos, sin Java/Kotlin. Todo el "código" de la app
es la `.so` Rust. El bootstrap Java de NativeActivity lo provee el
framework Android.

## Apps por portar (orden de menor a mayor fricción)

Las apps que **menos** se modifican al portar son las que ya tienen
poca interacción con teclado/mouse y mucho rendering:

1. **mirada-image-viewer-llimphi** — visor de imágenes, gestos = ok
2. **nahual-text-viewer-llimphi** — sólo scroll + zoom
3. **nahual-image-viewer-llimphi** — idem
4. **pluma-md-reader** — visor markdown, mismo patrón que la web
5. **chasqui-explorer-llimphi** — listas y tarjetas, taps obvios
6. **shuma-shell-llimphi** — teclado virtual, ya casi no usa shortcuts
7. **mirada-app-llimphi** — el compositor; touch desktop = problema UX

Las apps con paleta de comandos (nada, pluma-app full) son las
**últimas** porque su UX core (Ctrl+Shift+P, multi-pane splitter,
file picker) necesita ser repensada para touch.

## Próximos hitos

- **Tier 1.5**: hello-world con vello rasterizando un texto + figura
  (smoke test del stack raster completo en Android).
- **Tier 2**: portar `mirada-image-viewer-llimphi` — primer APK
  funcional con UI real.
- **Tier 3**: input handling proper (touch events, soft keyboard,
  back button), theming responsivo (dpi/density).
- **Tier 4**: distribución (Play Store internal track, F-Droid build
  reproducible).
