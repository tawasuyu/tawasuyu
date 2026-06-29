# SDD — Audito y migración de localización (i18n) de la suite

Documento de trabajo para terminar de hacer **multilingüe** la suite. La
infraestructura (`rimay-localize`) está sana; falta **cobertura**: ~20 apps UI
saltean el localizador y hardcodean español. Este SDD lista el trabajo pendiente
con pistas `archivo:línea` para retomarlo en otra máquina sin re-auditar.

**Fecha del audito: 2026-06-27.** Las pistas `archivo:línea` envejecen — re-`grep`
el string citado si el número no calza.

## Estado de la infraestructura (verificado)

- `shared/rimay-localize/locales/{es,en,qu}.ftl`: **paridad perfecta, 733 claves
  idénticas** en los tres idiomas, cero huecos. (Comando de verificación abajo.)
- **26 crates** ya consumen `rimay-localize` (cosmos, dominium, nakui-{explorer,
  sheet,ui}, mirada-{app,asistente,greeter}, shuma, supay-{app,doom}, media-app,
  nada, raymi, paloma, ayni, chaka-app, pluma-{editor,notebook}, chasqui-explorer,
  minga-explorer, wawa-{panel,explorer}, cosmos-modules, nahual-image-viewer).

### Override en runtime — traducir sin recompilar (2026-06-29)

Los catálogos siguen embebidos (`include_str!`) como **fallback garantizado**,
pero ahora se pueden **superponer en runtime** sin recompilar:

- `register_override(locale, ftl: &str)` — primitiva **agnóstica de fuente**:
  recibe el *contenido* `.ftl`, no una ruta. Pisa claves del embebido (la última
  capa gana) o registra un idioma nuevo si no hay embebido de esa lengua base.
- `load_overrides_from_dir(dir)` / `load_system_overrides()` — helpers host (usan
  `std::fs`). `init()` autocarga **sistema → usuario**: `/etc/wawa/locales/*.ftl`
  y `~/.config/wawa/locales/*.ftl` (misma raíz `wawa` que `wawa-config`). El
  nombre del archivo es la clave de locale (`de.ftl` → `de`, `es-PE.ftl` → `es-PE`).
  Precedencia: **usuario > sistema > embebido**.
- **Ángulo wawa:** wawa no tiene filesystem POSIX. La primitiva toma bytes, así
  que lo que corra ahí lee el `.ftl` de su almacén direccionado por contenido
  (akasha / `almacen.rs`) y llama a `register_override` — sin tocar `std::fs`. El
  helper de disco es host-only por diseño.

Así un distribuidor mete un idioma nuevo (p. ej. `de.ftl`) y un usuario corrige
claves sueltas, ambos sin recompilar.

Verificar paridad de catálogos:

```bash
cd shared/rimay-localize/locales
for l in en es qu; do grep -oE '^[a-zA-Z0-9_-]+ *=' $l.ftl | sed 's/ *=//' | sort -u > /tmp/keys_$l.txt; done
echo "en=$(wc -l < /tmp/keys_en.txt) es=$(wc -l < /tmp/keys_es.txt) qu=$(wc -l < /tmp/keys_qu.txt)"
comm -23 /tmp/keys_en.txt /tmp/keys_es.txt   # debe salir vacío
comm -23 /tmp/keys_es.txt /tmp/keys_qu.txt   # debe salir vacío
```

## Diagnóstico: la suite NO es uniformemente multilingüe

~20 apps UI hardcodean español (consistente, no mezclado es/en) por fuera del
localizador. Más 2 consumidoras con strings a medio migrar. Regla: las apps UI
muestran strings de usuario vía `rimay_localize::t(...)`, no literales `"..."`.

### ✅ Hecho — barrido 🔴/🟡 completo (2026-06-29)

Todas las apps que el audito listaba como 🔴/🟡 están localizadas vía
`rimay-localize`. Commits: nahual (`8164a36e`, `a929ac13`, `e821cbbc`),
`pluma-app` (`f01245a2`), `puriy` (`f36c842f`), claves `pata-*`
(`aab9373e`), y las 11 restantes —media-tube, uya, launcher, iniy,
hapiy, willay-panel, tinkuy, arje-card, chasqui-broker-explorer, churay,
sandokan-monitor— (`a0a4e813`). Paridad es/en/qu en 1684 claves. Quechua
aproximado en todo, **sujeto a revisión de un hablante**.

Además la infra soporta **override de catálogos en runtime** (sin recompilar)
— ver sección arriba.

### 🟠 Pendientes puntuales (lo que NO quedó)

- **`pata-llimphi` (fuente):** las **claves** `pata-*` están commiteadas
  (`aab9373e`, paridad ok) pero la **fuente** (las llamadas `t()` en
  `pata/src`) quedó SIN commitear por una colisión con otra sesión que editaba
  pata en paralelo (feature services/power-profile). Hay backup del trabajo del
  subagente en scratchpad. **Acción:** cuando esa sesión termine pata, rehacer
  la localización de `pata/src` sobre la versión final (las claves ya existen).
  Hasta entonces, esas 142 claves figuran sin uso en el código committeado.
- **`nahual-shell-core`** (otro crate): `FindMode::label` / `OpKind::label`
  siguen en español → el arg `{mode}` del overlay de búsqueda y los labels de la
  cola del shell no se traducen.
- **Sentinelas con tests exact-match** (dejadas literales a propósito;
  requieren separar *sentinela* de *etiqueta de display*, refactor fuera del
  barrido): `willay` `etiqueta_bucket` ("Hoy"/"Ayer"/…), `arje-card`
  `human_user_ns`/`human_cgroup`/`formatear_entrada`.
- **Títulos de ventana** (`App::title() -> &'static str`): quedan como marca en
  todas las apps (no se puede devolver el `String` de `t()` sin leak); donde hay
  item "Acerca de", el texto sí se localiza ahí.

> **Pendiente no auditado:** cobertura interna de las consumidoras viejas (las
> 26 originales) — grep de literales `"..."` en widgets que no pasen por `t(...)`.
> No se hizo un audito exhaustivo de esas.

### ⚪ Exentas (render-libs sin texto de usuario — NO tocar)

`cosmos-canvas-llimphi`, `dominium-canvas-llimphi`, `supay-render-llimphi`,
`takiy-app-llimphi`, `tullpu-app-llimphi`, `pluma-deck-recorrido-llimphi`,
`pluma-notebook-graph-llimphi`, `wawa-config-llimphi`, y los visores nahual
`hex / markdown / table / text / tree / video`. Reciben strings ya formados o solo
manejan claves técnicas (`"monospace"`, `"sun"`).

## Plan de migración

Por cada crate 🔴/🟡:

1. Agregar `rimay-localize` al `Cargo.toml` (si no está) — ver cualquiera de las 26
   consumidoras como patrón.
2. Por cada string de usuario hardcodeado: inventar una clave namespaced
   (`<crate>-<concepto>`, ej. `nahual-shell-esc-cierra`) y reemplazar el literal
   por `rimay_localize::t("clave")` (o la API de interpolación para `{var}`).
3. **Agregar la clave a los TRES `.ftl`** (`es`, `en`, `qu`) — la paridad (hoy
   1684 claves) es invariante de `main`; romperla es un bug. `es` = el texto que
   ya estaba; `en` y `qu` = traducción.
4. Re-verificar paridad con el comando de arriba.
5. `cargo check --workspace` (smoke test mínimo del repo).
6. Commit por crate o por subsistema: `feat(<scope>): localiza strings via rimay-localize`.

Este plan ya se aplicó a todo el 🔴/🟡 del audito (ver "Hecho" arriba). Lo que
queda son los **pendientes puntuales** (sección 🟠): la fuente de `pata`,
`nahual-shell-core`, las sentinelas con tests, y el audito de cobertura interna
de las consumidoras viejas.

## Cómo reproducir el audito (encontrar el gap completo)

```bash
cd /home/sergio/tawasuyu
# Universo de crates UI Llimphi
find . -name Cargo.toml | xargs grep -l 'name *= *"[a-z-]*-llimphi"' | sed 's|/Cargo.toml||' | sort > /tmp/all_llimphi.txt
# Consumidoras del localizador
grep -rl "rimay-localize" --include="Cargo.toml" . | sed 's|/Cargo.toml||' | sort > /tmp/consumers.txt
# Gap: UI que NO localiza (incluye render-libs exentas — filtrar a mano con la tabla de arriba)
comm -23 /tmp/all_llimphi.txt <(grep -- '-llimphi' /tmp/consumers.txt)
```
