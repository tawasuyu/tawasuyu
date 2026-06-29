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

### ✅ Hecho — subsistema nahual completo (2026-06-29)

Localizados vía `rimay-localize` (commits `8164a36e`, `a929ac13`, `e821cbbc`):
`nahual-image-viewer`, `nahual-audio-viewer`, `nahual-card-viewer`,
`nahual-file-explorer`, `nahual-archive-viewer`, `nahual-font-viewer`,
`nahual-gallery` y `nahual-shell-llimphi` (chrome + paleta + menubar +
contextual + modales + panel IA). Claves `nahual-*` en los tres `.ftl`
(paridad 902). **Pendiente del subsistema:** `nahual-shell-core` (otro crate)
— `FindMode::label` / `OpKind::label` siguen en español, así que el arg
`{mode}` del overlay de búsqueda y los labels de la cola no se traducen.

Además, la infra ahora soporta **override de catálogos en runtime** (sin
recompilar) — ver sección arriba.

### 🔴 Hardcodean español — enchufar al localizador

| Crate | Pista (archivo:línea — re-grep si no calza) | Ejemplo |
|---|---|---|
| `00_unanchay/pluma/pluma-app-llimphi` | view.rs:639-640 | `"quitar formato"`, `"cerrar"` |
| `00_unanchay/puriy/puriy-llimphi` | chrome.rs:46-73 | `"Deshacer"`, `"Rehacer"`, `"Cortar"` |
| `01_yachay/iniy/iniy-explorer-llimphi` | main.rs:374-389 | `"Deseleccionar"`, `"Recargar corpus"` |
| `01_yachay/tinkuy/tinkuy-llimphi` | lib.rs:721-737 | `"Pausar"`, `"Reanudar"`, `"Cambiar tema"` |
| `02_ruway/churay/churay-llimphi` | main.rs:730 | `"Instalar sugeridas"` |
| `02_ruway/hapiy/hapiy-llimphi` | main.rs:137 | `"Pulsá Capturar"` |
| `02_ruway/media/media-tube-llimphi` | main.rs:72 | `"media tube"` |
| `02_ruway/pata/pata-llimphi` | render/unidades.rs | `"Unidades"` |
| `02_ruway/uya/uya-llimphi` | main.rs | `"charla"` |
| `03_ukupacha/arje/arje-card-llimphi` | main.rs:1070, 793 | `"Refrescar"`, `"Aislamiento"` |
| `03_ukupacha/sandokan/sandokan-monitor-llimphi` | view_sistema.rs:172, view_unidades.rs:26 | `"Terminar"`, `"Sin unidades vivas"` |
| `shared/launcher-llimphi` | src/bin/tawasuyu-launcher.rs:110, 137 | `"tawasuyu · launcher"`, `"{n} apps descubiertas"` |
| `shared/willay/willay-panel-llimphi` | main.rs:114, 260 | `"Hoy"`, `"Copiado al portapapeles"` |

⚠️ El subsistema nahual ya está hecho (ver arriba). Próximo lote sugerido por
impacto: `pluma-app` / `puriy` / `iniy` / `tinkuy` → ruway misc → ukupacha/shared.

### 🟡 Mixtas — ya consumen el localizador pero dejaron strings sueltos

| Crate | Pista | Detalle |
|---|---|---|
| `02_ruway/chasqui/chasqui-broker-explorer-llimphi` | main.rs:621-622 | `"Refrescar probe"`, `"Limpiar timeline"` |

> **Pendiente no auditado:** las 26 consumidoras se confirmaron solo a nivel de
> dependencia, no de cobertura interna. El caso `nahual-image-viewer` sugiere que
> puede haber strings sueltos *dentro* de ellas. Falta un audito de cobertura por
> consumidora (grep de literales `"..."` en widgets que no pasen por `t(...)`).

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
3. **Agregar la clave a los TRES `.ftl`** (`es`, `en`, `qu`) — la paridad de 733 es
   invariante de `main`; romperla es un bug. `es` = el texto que ya estaba; `en` y
   `qu` = traducción.
4. Re-verificar paridad con el comando de arriba.
5. `cargo check --workspace` (smoke test mínimo del repo).
6. Commit por crate o por subsistema: `feat(<scope>): localiza strings via rimay-localize`.

Orden sugerido por impacto (el subsistema nahual ya está hecho): **pluma-app**
→ puriy / iniy / tinkuy → ruway misc → ukupacha/shared. La 🟡 restante
(`chasqui-broker-explorer`) es arreglo chico y conviene cerrarla al pasar.

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
