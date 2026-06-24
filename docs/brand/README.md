# Marca — tawasuyu

La identidad de la suite no se inventó para esta carpeta: **vive en el código
desde el principio**. Esta carpeta sólo la fija como assets reutilizables.

## La chakana

El glifo de marca es la **chakana** (cruz andina escalonada) con un **núcleo
luminoso** al centro — el mismo SVG que ya usan las landings (`web/*/index.html`,
clase `brand-chakana`) y el plano de cuadrantes de la portada. No es decoración:
es el cruce de los dos ejes del plano cartesiano sobre el que se organiza la
suite. Los cuatro brazos son los cuatro cuadrantes / las cuatro fases del ciclo
de la información:

| Cuadrante | Fase | Color de tema |
|---|---|---|
| `00_unanchay` | **PERCIBIR** | `#B9C9E8` |
| `01_yachay`   | **CONOCER**  | `#E8C97A` |
| `02_ruway`    | **HACER**    | `#E89B6E` |
| `03_ukupacha` | **RAÍZ**     | `#8FB58C` |

## Paleta

Tomada literalmente de `02_ruway/llimphi/llimphi-theme::Theme::dark()` — la misma
que pinta el chrome del escritorio, las landings y este wallpaper:

| Rol | Hex |
|---|---|
| Fondo app | `#0E1016` |
| Panel | `#161A24` |
| Texto | `#D6DEE8` |
| Texto atenuado | `#8C98AA` |
| Acento (núcleo) | `#6E8CDC` |

## Tipografía

Wordmark en **JetBrains Mono** Light con tracking generoso — la suite es un
sistema de herramientas, y el monoespaciado lo dice sin adornos. Fallback:
Noto Sans.

## Assets

- `wallpaper.svg` — fuente vectorial del fondo (chakana + plano + verbos + wordmark).
- `wallpaper-{1920x1080,2560x1440,3840x2160}.png` — derivados rasterizados.
- `chakana.svg` / `chakana-512.png` — marca suelta (favicon, README, web).
- `logo-suite.svg` / `logo-suite-256.png` — logo de app (ícono redondeado + chakana).

Regenerar los PNG: `scripts/build-brand.sh` (requiere `rsvg-convert`).

## Runtime: el crate `shared/marca`

La identidad que las **apps muestran en vivo** (logo + nombre + tagline + acento de
suite/hammer/wawa, p. ej. la pantalla de bienvenida de `churay`) vive en el crate
[`shared/marca`](../../shared/marca), no acá. Esta carpeta es la **fuente vectorial**
(wallpaper + chakana + logo) y la guía; `marca` es el **consumo en runtime**.

`marca` **embebe** (`include_bytes!`) el set: el logo de cada marca
(`assets/{suite,hammer,wawa}.png` — el de la suite **es la chakana**) y el
**wallpaper** del escritorio (`assets/wallpaper.png`). Viaja dentro de cada
binario que la consuma; no hay que instalar nada aparte.

API: `Brand::Suite.image()` (logo), `Brand::Suite.meta()` (nombre/tagline/acento),
`marca::wallpaper()` (fondo). Todas resuelven primero un **override por disco sin
recompilar** — dejá `suite.png` / `wallpaper.png` en `$TAWASUYU_MARCA` o
`~/.config/tawasuyu/marca/` y gana sobre el embebido. Es el gancho del rebrand.

Para regenerar los assets embebidos desde las fuentes vectoriales:

```bash
scripts/build-brand.sh
cp docs/brand/logo-suite-256.png    shared/marca/assets/suite.png
cp docs/brand/wallpaper-2560x1440.png shared/marca/assets/wallpaper.png
```

## Fondo por defecto de mirada

No hace falta configurar nada: con `wallpaper_path` vacío (el default), mirada usa
el **wallpaper de marca** (`marca::wallpaper()`, fit `fill`) como fondo del
escritorio; sólo cae a un gradiente sobrio si el PNG no decodificara. El usuario
puede pisar el default poniendo su propia imagen en `~/.config/mirada/config.ron`:

```ron
wallpaper_source: "local",
wallpaper_path: "/home/<user>/fondos/lo-que-sea.png",
wallpaper_fit: "fill",
```
