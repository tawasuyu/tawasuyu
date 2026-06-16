# Auditoría de arquitectura — frontends vs. cores agnósticos

**Fecha:** 2026-06-15
**Disparador:** la corrección de `shuma` (estaba "todo junto standalone y no agnóstico") expuso una
clase de violación; este barrido busca el mismo defecto en el resto del workspace.

**Reglas auditadas** (CLAUDE.md):
- **R1** — un dominio = un crate raíz con subcrates plugin; splittear crates > ~1500–2000 LOC.
- **R2** — las UIs son frontends intercambiables sobre `*-core` **agnósticos**; la lógica de dominio
  no sabe quién la pinta. Test: *si cambiás el frontend a web/CLI, ¿podrías reusar la lógica?*
  Si la lógica **sólo** vive en el crate de UI → violación.

**`shuma` queda fuera de este documento** — lo está corrigiendo otro agente.

## Veredicto global

El workspace está mayormente sano. **Cero dependencias GPUI** en todo el árbol (los hits de `grep`
son comentarios/`description` históricos). Casi todos los dominios ya tienen su `*-core` agnóstico.
Lo que sigue son los casos donde **lógica de dominio quedó atrapada en un frontend** — el mismo
defecto de clase que tenía shuma.

| # | Crate | Severidad | Tipo | Estado |
|---|-------|-----------|------|--------|
| 1 | `02_ruway/nahual/nahual-shell-llimphi` | **ALTA** | (a)+(c) fat binary, motor de búsqueda+IA atrapado, sin `-shell-core` | 🟡 find + `ops` + helpers de IA extraídos a `nahual-shell-core`; queda sólo convertir bin→lib hosteable |
| 2 | `02_ruway/takiy/takiy-app-llimphi` (`model/`) | **MEDIA** | (a) `EditorState`/undo-redo agnóstico atrapado en el binario | ✅ hecho — extraído a `takiy-editor-core` |
| 3 | `01_yachay/cosmos/cosmos-app-llimphi/src/astrocarto.rs` | **MEDIA** | (a) astronomía (JD/GMST/oblicuidad/líneas) recalculada en la UI | ⬜ pendiente |
| 4 | `03_ukupacha/sandokan/sandokan-monitor-llimphi` (modo Sistema) | **MEDIA** | (a)+(b) procfs/%CPU/árbol/señales sin core | 🟡 `procfs` (lee /proc + señales) → nuevo `sandokan-sysmon-core`; falta mover `SysProc`+helpers de `sistema.rs` |
| 5 | `02_ruway/media/media-app/src/playlist.rs` | **MEDIA** | (a) **duplica** `media-core::playlist` (reimplementación divergente) | ⬜ pendiente |
| 6 | `00_unanchay/khipu/khipu-app/src/map.rs` | MEDIA | (a) `place_note` (anclaje semántico); `gravity_layout` del core sin usar | ✅ hecho — `SemanticField::anchor_new` |
| 7 | `02_ruway/chasqui/chasqui-broker-explorer-llimphi` | BAJA | (a) `diff_matches`/timeline de salud del broker atrapado | ✅ hecho — `card-handshake::health` |
| 8 | `02_ruway/pata/pata-llimphi/src/sampler.rs` | BAJA | (a) efemérides (`astro_from_jd`) atrapadas; `pata-core` es agnóstico | ✅ hecho — `pata-core::astro` |
| 9 | `00_unanchay/khipu/khipu-app/src/estado.rs` | BAJA | (a) embedder fallback (`embed`) atrapado | ✅ hecho — `khipu_gravity::local_embed` |
| 10 | `02_ruway/nahual/nahual-font-viewer-llimphi` | BAJA | (a) parseo TTF en el frontend; `viewer-core` sin módulo `font` | ✅ hecho — `nahual-viewer-core::font` |
| 11 | `03_ukupacha/wawa-explorer/wawa-explorer-llimphi` | BAJA | (a) `resolver_iface` (lee `/sys/class/net/`) atrapado | ✅ hecho — `wawa-explorer-aoe` |
| 12 | `03_ukupacha/arje/arje-card-llimphi` | BAJA | (a) `detect_units`/`resumir_atestacion` atrapados | 🟡 `resumir_atestacion`→`arje-brain::audit`; `detect_units` se deja (ver nota) |

Tipos: **(a)** lógica de dominio atrapada en el frontend · **(b)** sin `*-core` agnóstico ·
**(c)** binario monolítico sin separar lib+bin · **(d)** deps GPUI.

---

## Detalle

### 1. `nahual-shell-llimphi` — ALTA (a)+(c)
Bin-only (sin `lib.rs`), `main.rs` de 480 LOC, **7754 LOC** de src. Es el anti-patrón opuesto a la
corrección de shuma. No existe `nahual-shell-core`. Atrapado en el binario:
- `src/find.rs` (479 LOC): motor de búsqueda **léxico + semántico por embeddings**
  (`run_find`, `grep_first`, `run_find_semantic`, `build_index`, `collect_candidates`, `cosine_slices`)
  — con tests unitarios propios.
- `src/ai.rs` (270 LOC): armado de contexto + prompts LLM + batch-rename (`propose_names`).
- `src/ops.rs` (204 LOC): cola de operaciones de archivo (las mutaciones reales sí delegan en
  `nahual_source_core::SourceMut`).

**Remediación:** crear `nahual-shell-core` agnóstico (find léxico+semántico tomando un iterador/closure
de IO, armado de prompts, modelo de navegación, cola de ops). Convertir `nahual-shell-llimphi` en lib
(`Model`/`Msg`/`App`/`run()`) + `main.rs` fino, espejando `shuma-shell-llimphi`.

### 2. `takiy-app-llimphi` — MEDIA (a)
`src/model/` (`mod.rs` 354 + `apply.rs` 830 + `describe.rs` 128 ≈ 1300 LOC sin tests) se autodocumenta
como *"lógica pura del editor: cero audio, cero UI… El binario Llimphi le manda `EditMsg`s"*. Es
`EditorState` + `apply()` con undo/redo + operaciones de notas/pistas/mixer/tonalidad/automación,
**agnóstico de GUI pero físicamente atrapado en el crate binario**. `takiy-core` existe pero sólo tiene
teoría musical (`Score`/`Track`/`Pitch`).

**Remediación:** extraer `model/` a `takiy-editor-core`; `takiy-app-llimphi` lo consume y queda con
`update`/`view`/`paint`/`chrome`. Bajo riesgo: ya está limpio internamente.

### 3. `cosmos-app-llimphi/src/astrocarto.rs` — MEDIA (a)
El tile AstroCarto recalcula astronomía cruda en la app: `julian_day_utc` (l.23), `gmst_deg` (l.41),
`ecliptic_to_equatorial` con `OBLIQUITY=23.4393°` fijo (l.51), ángulo horario orto/ocaso (l.367).
Todo esto ya existe en `cosmos-time`/`cosmos-coords`/`cosmos-rise-set`/`cosmos-skywatch`. El cómputo de
líneas MC/IC/Asc/Desc sólo vive en la UI (no reusable desde `cosmos-web`/`cosmos-cli`).

**Remediación:** extraer la proyección a `cosmos-astrocartography` (o a `cosmos-coords`/`cosmos-render`)
que reciba posiciones de `cosmos-skywatch` y devuelva polilíneas; `astrocarto.rs` sólo pinta.

### 4. `sandokan-monitor-llimphi` (modo Sistema/htop) — MEDIA (a)+(b)
La mitad "plano de control" usa bien `sandokan_monitor_core::observe`. La mitad "Sistema/htop" es
lógica de dominio pura sin core:
- `src/procfs.rs` (268 LOC): parseo crudo de `/proc` (`scan`, `parse_one`, `cpu_stat`, `meminfo_kb`) +
  envío de señales (`signal` con `nix::kill`).
- `src/sistema.rs` (l.44–280): %CPU por deltas (`ingest_system`), árbol padre/hijo (`flatten_tree`,
  `subtree_pids`), `sort_system`, `proc_matches`.

**Hecho:** `procfs.rs` (barrido de `/proc`, jiffies CPU/RAM, `signal` por `nix`) → nuevo crate agnóstico
`shared/sandokan/sandokan-sysmon-core`; el frontend lo re-exporta como `crate::procfs::*` (shim) y se le
quitaron las deps `nix`/`libc` (ya no las usa directo). **Falta (follow-up):** mover `SysProc` (hoy en
`modelo.rs`) y los helpers puros de `sistema.rs` (`flatten_tree`/`subtree_pids`/`proc_matches`/`push_capped`
+ la derivación de %CPU de `ingest_system`) al core — requiere mover `SysProc` primero (lo referencian 4
archivos del frontend) y desacoplar `ingest_system` del `Model`.

### 5. `media-app/src/playlist.rs` — MEDIA (a, duplicación)
Define su propio `struct Playlist` y reimplementa `cycle_repeat`/`toggle_shuffle`/`shuffle_order` (LCG
propio)/`next`/`prev`/auto-advance — **todo ya existe en `media-core::playlist`** (`Repeat`, `Playlist`,
`cycle_repeat`, `toggle_shuffle`, `shuffle_order`, `next`, `prev`). Riesgo de divergencia.

**Remediación:** reemplazar el `Playlist` local por wrapper sobre `media_core::playlist::Playlist`;
dejar en el frontend sólo `decoders: Vec<LoadedTrack>` + índice. Borrar las reimplementaciones.

### 6. `khipu-app/src/map.rs` — MEDIA (a)
`place_note()` (l.90) reimplementa anclaje semántico (baricentro por afinidad coseno + relajación +
golden-angle). Existe `SemanticField::gravity_layout()` en `khipu-gravity` (l.255) pero **nunca se
llama** (el doc de `main.rs:10` está stale). **Remediación:** mover el anclaje incremental a
`khipu-gravity` (`anchor_new(field, placed, id) -> (f32,f32)`); corregir el doc de `main.rs`.

### 7. `chasqui-broker-explorer-llimphi` — BAJA (a)
`diff_matches()` (l.673) es una máquina de estados de "salud del broker" (compara `MatchKey`s entre ticks
→ eventos Available/Lost a un timeline). Reusable; una CLI lo duplicaría. **Remediación:** mover
`diff_matches`+`TimelineEntry` a `chasqui-broker` (o `chasqui-broker-client`). *(Contraste:
`chasqui-explorer-llimphi` está OK — delega a `card_sidecar`/`chasqui_card::query`.)*

### 8. `pata-llimphi/src/sampler.rs` — BAJA (a, soft)
`astro_from_jd()` (l.288) + `MES_SINODICO`/`LUNA_NUEVA_REF_JD` es matemática pura no_std. `pata-core` es
agnóstico y declara al wawa-kernel como segundo consumidor → reimplementaría esto. **Remediación:** mover
`astro_from_jd` + constantes a `pata-core` (módulo `astro`). *(Resto de `sampler.rs`/`weather.rs`/`cava.rs`
es host-IO legítimo del frontend.)*

### 9. `khipu-app/src/estado.rs` — BAJA (a)
`embed(text, dim)` es un embedder local (hash FNV de trigramas) atrapado en la app. **Remediación:**
moverlo a `khipu-gravity` o a `rimay-verbo-mock` como `Provider` de fallback.

### 10. `nahual-font-viewer-llimphi` — BAJA (a)
Parsea TTF/OTF con `ttf-parser` dentro del `-llimphi`; **no** depende de `nahual-viewer-core`, que además
**no tiene módulo `font`** pese a que su `description` lo afirma. **Remediación:** mover el parseo de
metadatos a `nahual-viewer-core::font` (dejar outlines→`BezPath` en el `-llimphi`); corregir la
`description` de viewer-core.

### 11. `wawa-explorer-llimphi/src/main.rs` — BAJA (a)
`resolver_iface` (l.889, lee `/sys/class/net/`, ~30 LOC) atrapado en la UI. **Remediación:** mover a
`wawa-explorer-aoe`.

### 12. `arje-card-llimphi/src/main.rs` — BAJA (a)
`detect_units` (l.421, scan del card store) y `resumir_atestacion` (l.270, resumen del audit) atrapados.
**Hecho:** `resumir_atestacion` + `AttestSummary` → `arje-brain::audit` (ya era dep; sin árbol nuevo).
**`detect_units` se deja a propósito:** el comentario en `main.rs:391-393` documenta una decisión
deliberada — `cards_dir` se replicó "para no arrastrar el árbol de deps de arje-compat por 4 líneas".
Mover `detect_units` a `arje-compat` forzaría al frontend a depender de zbus/hickory-resolver/etc., una
regresión de higiene de deps contra una decisión consciente. Respetamos esa decisión (CLAUDE.md: si el
target contradice cómo se describió, surgir el conflicto en vez de proceder).

---

## Nits de R1 (no violan R2, opcionales)
Varios frontends son binarios de un solo `main.rs` grande sin separar lib hosteable
(`chaka-app-llimphi`, `pluma-app`, `cosmos-app-llimphi`, `iniy-explorer-llimphi`, `wawactl` 1467,
`wawa-panel-llimphi` 1410, `arje-card-llimphi` 1328). No es violación de agnosticismo; convertirlos a
lib+bin fino (patrón shuma) es mejora de estilo, baja prioridad.

## Ejemplos limpios (referencia del patrón correcto)
`pineal`, `pluma`, `dominium`, `tinkuy`, `paloma`, `raymi`, `tullpu`, `agora`, `shared/launcher-*` y la
mayoría de los `nahual-*-viewer-llimphi` delegan correctamente a cores agnósticos.
