# UNIFICACION.md — nahual, el front universal nivel Directory Opus

> Plan de implementación. Snapshot: 2026-06-11. Continúa **BRAHMAN.md Fase 3** (espina de
> exploradores completa) → **Fase 4** (file manager pleno + AppBus vivo + absorción de los
> exploradores sueltos). Cuando contradiga a un SDD de dominio, el SDD gana; cuando contradiga
> a BRAHMAN sobre "la espina", BRAHMAN gana. Lenguaje denso, para IA y para el autor.

## 0. Tesis

No se construye un explorador nuevo. **`nahual-shell` ES el front universal** y la unificación de
datos **ya existe**: `nahual-source-core::Source` con cuatro adapters (POSIX · wawa · nouser ·
minga) montados por `Navigator`, y `viewer_registry` despacha visor por contenido. Lo que falta es
elevar ese front de "lista + un visor" a un **file manager nivel Directory Opus** y volver **vivo el
AppBus** para que cualquier hoja se abra con cualquier app de la suite. La proliferación de
exploradores (nada, chasqui-explorer, wawa-explorer, el navegador de pata) se cura **absorbiéndolos
en este front o degradándolos a productores de la misma espina**, no manteniéndolos en paralelo.

Norte de una frase: **una sola ventana que abre cualquier cosa de la suite —un dir POSIX, una
imagen wawa, una Mónada de nouser, un repo minga— con dual-pane, columnas, operaciones de archivo,
y "abrir con" hacia las ~30 apps del repo.**

## 1. Estado real (lo que NO hay que rehacer)

| Pieza | Estado | Path |
|---|---|---|
| Trait `Source` agnóstico + `Node` | ✅ vivo (4 adapters) | `02_ruway/nahual/nahual-source-core/src/lib.rs` |
| `Navigator` (estado de navegación sobre `dyn Source`) | ✅ vivo | `.../navigator.rs` |
| Front monta los 4 mundos (POSIX·wawa·nouser·minga) | ✅ vivo (BRAHMAN F3 paso E) | `nahual-shell-llimphi/src/main.rs` |
| `viewer_registry` data-driven (13 visores, ranking lens/mime/priority) | ✅ vivo | `.../viewer_registry.rs` |
| Despacho por contenido (`shuma-discern` → `pick`) | ✅ vivo | `.../main.rs:load_for` |
| `AppRegistry` + `AppEntry{handles, launch}` + `open()/open_with()/handlers_for()` | ✅ vivo | `shared/app-bus/src/lib.rs` |
| Gancho `external_handler_for(registry, discernment)` (open-with externo) | ✅ existe, **sin cablear** | `.../viewer_registry.rs` |
| Widgets: splitter, tabs, tree, list, table, breadcrumb, context-menu, text-input, menubar, panes(BSP) | ✅ existen | `02_ruway/llimphi/widgets/*` |
| Pantallazo headless de la unificación archivos↔Mónadas | ✅ nuevo (5c7042f8) | `nahual-shell-llimphi/examples/pantallazo_monadas.rs` |

**Conclusión:** la arquitectura aguanta. Fase 4 es **wiring + extensión acotada de trait + features
de UI**, no reconstrucción.

## 2. Gap contra Directory Opus (lo que SÍ falta)

Dopus = el file manager más potente de Windows. Mapeo cada feature a su estado y a la pieza que la
realiza:

| Feature Dopus | Estado | Realiza con |
|---|---|---|
| Dual-pane (dos exploradores lado a lado) | ❌ | `splitter` (existe) + 2 `Navigator` en el Model |
| Tabs por pane | ❌ | `widget-tabs` (existe) + `Vec<Navigator>` por pane |
| Árbol lateral persistente | ◑ (tree existe, no integrado al shell) | `widget-tree` |
| Breadcrumbs navegables | ◑ (hay header de texto, no clicable) | `widget-breadcrumb` (existe) |
| Vista detalle: columnas (nombre/tamaño/mtime/tipo/permisos) ordenables | ❌ | extender `widget-table` con sort+headers clicables |
| Vistas conmutables (lista / detalle / iconos) | ❌ | enum `ViewMode` + 3 renders |
| Selección múltiple (Ctrl/Shift-click, marquee) | ❌ | `selected: BTreeSet<usize>` en `Navigator` |
| Operaciones: crear/borrar/renombrar/mover/copiar | ❌ | **extender trait** (`SourceMut`) + acciones |
| Cola de operaciones con progreso/cancelar | ❌ | `OpQueue` en el Model + `Handle::spawn` |
| Batch rename (patrón/regex) | ❌ | módulo `rename_batch` + preview |
| Filter bar (filtrar por nombre/glob en vivo) | ❌ | `text-input` + filtro sobre `children` |
| Buscar archivos (find por nombre/contenido) | ❌ | reusar `llimphi-module-fif` de nada |
| Viewer pane (preview en vivo del seleccionado) | ✅ | ya existe (el panel derecho actual) |
| "Abrir con…" (elegir app) | ◑ gancho listo | AppBus `handlers_for` + context-menu |
| Archivos como carpetas (montar zip/tar) | ◑ (hay archive-viewer read-only) | adapter `ArchiveSource` |
| Labels/colores por archivo | ❌ | sidecar `sled` de metadata (`label_store`) |
| Favoritos / lugares | ❌ | `places` persistido (sled/toml) |
| Toolbar configurable | ❌ (baja prioridad) | declarativo desde toml |
| Folder formats (recordar vista por carpeta) | ❌ | mapa `path→ViewPrefs` persistido |
| Sync/compare de carpetas | ❌ (extrapolación) | doble `Navigator` + diff de árboles |
| FTP/cloud/remoto | ◑ extrapolación | adapter `NetSource` sobre card-net (CAPA 1) |
| Scripting | ◑ | acciones Rhai (pluma ya embebe Rhai) |

## 3. Decisiones de arquitectura

**D1 — Un solo binario: `nahual-shell` crece, no nace otro.** El "file manager" es nahual con más
features. `nada` queda como **editor** (open-with de texto/código), no como file manager.

**D2 — Partir el trait en lectura/escritura.** `Source` sigue read-only (lo cumplen los 4 mundos).
Las operaciones van en un trait separado para no obligar a wawa/minga (CAS inmutable) a fingir
escritura:
```rust
pub trait SourceMut: Source {
    fn create_dir(&self, parent: &NodeId, name: &str) -> io::Result<NodeId>;
    fn create_file(&self, parent: &NodeId, name: &str) -> io::Result<NodeId>;
    fn delete(&self, id: &NodeId) -> io::Result<()>;
    fn rename(&self, id: &NodeId, new_name: &str) -> io::Result<()>;
    fn move_into(&self, id: &NodeId, new_parent: &NodeId) -> io::Result<NodeId>;
    fn copy_into(&self, id: &NodeId, new_parent: &NodeId) -> io::Result<NodeId>;
    fn write(&self, id: &NodeId, bytes: &[u8]) -> io::Result<()>;
}
```
`PosixSource` lo implementa; el resto no. El shell consulta `fn as_mut(&self) -> Option<&dyn
SourceMut>` (default `None`) y **gatea la UI**: sin `SourceMut`, los ítems de operación salen
deshabilitados (frontera honesta, igual que wawa).

**D3 — `Node` gana metadata opcional, sin romper a nadie.** Campos `Option`: las fuentes que no
tienen el dato devuelven `None` y la columna sale "—".
```rust
pub struct Node {
    pub id: NodeId, pub name: String, pub is_container: bool,
    pub size: Option<u64>,          // bytes
    pub mtime: Option<u64>,         // epoch ms
    pub kind: NodeKind,             // File|Dir|Symlink|Archive|Synthetic
    pub mime_hint: Option<String>,  // si la fuente ya lo sabe (evita re-discern)
}
```

**D4 — El Model del shell pasa a multi-pane / multi-tab.**
```rust
struct Pane { tabs: Vec<Navigator>, active: usize, view: ViewMode, sel: BTreeSet<usize>, sort: SortKey }
struct Model { panes: [Pane; 2], focus: usize /*0|1*/, dual: bool, ops: OpQueue, places: Vec<Place>, ... }
```
`dual=false` ⇒ un solo pane (modo simple actual, compatible). Operaciones copy/move usan
`focus`→el otro pane como destino (gesto Dopus clásico).

**D5 — AppBus vivo = cablear `external_handler_for`.** El context-menu de una hoja ofrece
"Abrir con <app>" por cada `AppEntry` en `handlers_for(mime)`; activar = `AppEntry::open(path)`.
Hoja no-POSIX (Mónada/wawa) ⇒ `nav.read(id)`→tempfile→`open(tempfile)`. Doble-clic abre con el
**default** (primer handler o el visor in-process si no hay app externa). Esto realiza BRAHMAN
"widgets/apps hablan por la espina" para el caso app↔archivo.

**D6 — Persistencia lateral en sled** (un solo árbol `nahual-state`): labels, places, folder
formats, recents. No toca las fuentes (que pueden ser read-only).

**D7 — Evidencia, no aserción (Regla 3 + lección "diente").** Cada fase entrega un
`examples/pantallazo_*.rs` headless que renderiza la feature a PNG. No se declara "listo" sin el
PNG citado.

## 4. Plan por fases

Cada fase es un bloque funcional commiteable (`cargo check --workspace` verde + pantallazo).

### F4.0 — Cimientos del trait y la metadata  *(refactor base)*  ✅ HECHA (5f095963)
- `Node` gana `size/mtime/kind/mime_hint` (D3); adapters los llenan (POSIX: stat real; wawa:
  size del objeto; nouser: stat de miembros; minga/wawa: kind).
- Trait `SourceMut` (D2) + `Source::writable()->Option<&dyn SourceMut>` (se nombró `writable()`
  en vez de `as_mut()` por claridad). `PosixSource` lo implementa (rename cross-fs → copiar+
  borrar; copy recursivo; `ruta_libre` anti-pisado). wawa/minga read-only (gateo honesto).
- **Entrega:** +7 tests (22 verde con `--features nouser,minga`).

### F4.1 — Vista detalle con columnas ordenables  *(la cara Dopus)*  ✅ HECHA (a758aae1)
- Widget NUEVO `llimphi-widget-detail-table` (el `widget-table` era editable, no servía):
  grilla read-only, columnas flex/fijas, headers clicables `on_sort(col)` con flecha ▲/▼.
- `ViewMode{List,Details}` en `Navigator` (Icons queda para F4.8); `Details` pinta nombre/
  tamaño/mtime/tipo desde la metadata de `Node` (human_size + fecha civil sin deps).
- Orden por columna en `Navigator` (`SortKey`+`SortDir`, contenedores siempre agrupados arriba,
  `set_sort` preserva selección por id). Filtro vivo (`set_filter`/`visible()`, up/down saltan
  lo filtrado). Shell: `v` alterna lista/detalle, `/` filtra, click en header reordena.
- **Entrega:** `pantallazo_detalle.rs` (raíz del repo, detalle ordenado por Tamaño ▼). +3 tests
  Navigator (25 verde).

### F4.2 — Dual-pane + breadcrumbs + unificación POSIX  *(el chasis Dopus)*  ✅ HECHA (14ea04d3·5cc33c95·22f53d72)
- **F4.2a (linchpin)** — POSIX unificado sobre `Navigator`: el shell pasa de `explorer:
  FileExplorerState` + `mounted:Option<Navigator>` a un solo `nav_stack:Vec<Navigator>` ([0]=POSIX
  base anclada en `/`, arrancada en el cwd vía `Navigator::open_at` con la cadena de ancestros;
  montar empuja, desmontar saca). Todos los handlers operan sobre `cur()`. **POSIX hereda detalle/
  orden/filtro** (antes lista plana). `+current_id`.
- **F4.2b** — breadcrumbs clicables (`widget-breadcrumb`): `Navigator::ancestors()` + `ascend_to(depth)`;
  cada segmento sube a su nivel; prefijo `⊟<fuente>` sobre montadas.
- **F4.2c** — panel doble: `Model{panes:[Pane;2], focus, dual}`; `d` alterna panel+visor ↔ panel|panel,
  `Tab` cambia foco; filas/encabezados/breadcrumbs llevan el índice de panel (`SelectIn`/`SortByIn`/
  `BreadcrumbIn`); panel enfocado resaltado.
- **Entrega:** `pantallazo_detalle` (POSIX profundo + breadcrumb), `pantallazo_dualpane` (raíz en
  detalle | widgets en lista). +3 tests Navigator (27 verde).
- **Pendiente (F4.2d, no bloqueante):** tabs por panel (`Ctrl+T`/`Ctrl+W`) y árbol lateral
  (`widget-tree`) sincronizado. El chasis (dual-pane) ya soporta agregarlos.

### F4.3 — Operaciones de archivo + cola  *(el músculo)*  ✅ HECHA
- Selección múltiple (`BTreeSet<NodeId>` por panel, marca con `Insert`) + acciones: New folder/file
  (`F7`/menú), Rename (`F2`, prompt modal con el nombre actual), Delete (`Supr`, **diálogo de
  confirmación** sí/no), Copy/Move pane→pane (`F5`/`F6`, sólo en dual). Todo cableado además al
  menú principal (Archivo) y al contextual.
- `OpQueue` (`ops.rs`): cada operación es un job async (`Handle::spawn`) que reconstruye una
  `PosixSource` y corre por `SourceMut`; al terminar reentra con `Msg::OpFinished` y recarga ambos
  paneles (`Navigator::reload`, conserva selección por id). Panel inferior colapsable lista los jobs
  con su estado (`⋯`/`✓`/`✗` + error). "Limpiar" olvida los terminados.
- Gateo por `Navigator::writable()` (D2): sobre fuentes montadas read-only (wawa/minga/nouser) los
  ítems salen en gris / los atajos no disparan.
- **Entrega:** `pantallazo_ops.rs` — lista con filas marcadas + cola con un copy en curso y un rename
  terminado + prompt de renombrar. +5 tests (`ops`: 3, `navigator`: reload/select_id/writable).
- **Pendiente menor** (no bloqueante): papelera XDG real (hoy Delete borra directo tras confirmar);
  diálogo de conflicto skip/overwrite/rename (hoy copy/move auto-renombra con `ruta_libre`, sin
  pisar); Ctrl/Shift-click para marcar (hoy la marca es por teclado, `Insert`); cancelar un job en
  vuelo.

### F4.4 — AppBus vivo: "Abrir con…" hacia toda la suite  *(la integración)*  ✅ HECHA (291541fc)
- Context-menu "Abrir con <app>" desde `AppRegistry::handlers_for(mime discernido)`; activar =
  `AppEntry::open(path)` (hoja no-POSIX → tempfile con su nombre). + "Editar en Nada" + "Abrir
  terminal aquí" (shuma). Las opciones se precomputan al abrir el contextual (no toca disco en
  render).
- app-bus: `handles_mime` soporta **prefijos** (`image/` matchea `image/*`); `default_entries()`
  = catálogo de la suite con sus mimes (en código, funciona sin sembrar config);
  `AppRegistry::with_defaults()` funde defaults+disco+`.desktop` (handles se unen); `reveal(path)`
  = Reveal in nahual (recíproco); el shell honra `argv[1]` como cwd.
- **Entrega:** `pantallazo_openwith.rs` — contextual sobre un `.flac` → Abrir con Media/Takiy
  (enrutado por `audio/flac`). +2 tests app-bus (15 verde).
- **Pendiente menor** (no bloqueante): persistir "app predeterminada" en sled (D6) y sembrar las
  Cards de apps en `assets/apps/*.toml` (hoy los defaults viven en código). Registrar visores como
  Cards on-disk ya está cubierto por BRAHMAN F2a (`discover_viewer_cards`).

### F4.5 — Batch rename · labels · favoritos · folder formats  *(power-user)*  ◐ EN CURSO
- **F4.5a (batch rename) ✅ HECHA** — patrón con tokens `{name}`/`{ext}`/`{n}` (contador 1-based) +
  preview tabular `viejo → nuevo` con detección de colisiones (rojo) antes de aplicar. Disparado por
  `F2` con marca múltiple (sin marca, `F2` = renombrado simple), o "Renombrar por lote…" del
  contextual. Al aplicar encola un `OpKind::Rename` por objetivo cuyo nombre cambie (reusa la cola
  F4.3). `aplicar_patron` + `BatchRename`; +4 tests. Pantallazo: `pantallazo_power.rs`. *(Pendiente:
  regex/Rhai en el patrón, padding del contador `{n:3}`.)*
- **Pendiente:** Labels/colores por archivo (sled), columna y tinte de fila.
- **Pendiente:** Favoritos/places (sidebar) + recents; folder formats (recordar `ViewMode`/`sort` por
  path). Todo esto vive en `nahual-shell-llimphi/src/state.rs` (sled) — el bloque F4.5b.

### F4.6 — Adapters extra como Source  *(extrapolación de fuentes)*
- `ArchiveSource`: montar `.zip/.tar/.tar.gz` como árbol navegable (reusa el reader del
  `archive-viewer`); `read` extrae la entrada. Read-only.
- `NetSource` (opt-in, sobre **card-net** CAPA 1): navegar contenido remoto direccionado por
  `DhtKey` (el "FTP/cloud" de Dopus, pero P2P y soberano). Read-only primero.
- Doble-clic en un `.zip` POSIX → auto-monta `ArchiveSource` (como ya hace con `.img`→wawa).
- **Entrega:** `pantallazo_archive.rs` — un .tar.gz montado como carpeta + un archivo leído.
- ~400 LOC.

### F4.7 — Absorción y retiro de exploradores sueltos  *(saldar la deuda)*
- **`pata`**: su `nouser.rs` (reimplementa el query a chasqui) pasa a consumir
  `nahual-source-core::NouserSource` — una sola ruta al dato de Mónadas. (O, si pata quiere el
  daemon vivo y nahual el escaneo local, factorizar el camino común; decidir con evidencia.)
- **`chasqui-explorer-llimphi`**: retirar — es un subconjunto de `nahual` montando `NouserSource`.
  Dejar `chasqui-broker-explorer-llimphi` (debug del broker, otra cosa).
- **`wawa-explorer-llimphi`**: degradar a "abrir `.img` en nahual" (ya hay `WawaImgSource`). Mantener
  `wawa-explorer-core` (lo usa el adapter) y `-aoe` (fetch peers); la UI suelta se retira o queda
  como demo. El "fetch from peers" se reexpone como acción de la fuente wawa en nahual.
- **`nada`**: queda como editor; se integra como open-with (D5) y "Reveal in nahual" recíproco.
- **Entrega:** `cargo check --workspace` verde tras los retiros; pata renderiza Mónadas por la
  fuente común (pantallazo).
- ~300 LOC (mayormente borrado/mudanza).

### F4.8 — Pulido Dopus restante  *(opcional, según uso)*
- Sync/compare de carpetas (doble Navigator + diff visual).
- Toolbar configurable declarativa; scripting de acciones Rhai.
- Vista iconos con thumbnails (reusa decoders de los viewers).
- Sin pantallazo único: cada sub-feature trae el suyo.

## 5. Integración con TODAS las apps (mapa content-type → app)

El AppBus ya keya por MIME. Sembrar una Card toml por app en `shared/app-bus/assets/apps/`. Mapa
propuesto (handler primario en **negrita**):

| Contenido / dominio | MIME(s) | Apps que lo abren |
|---|---|---|
| texto / código | `text/*`, `text/x-rust`… | **nada** (editar), pluma (doc rico) |
| markdown | `text/markdown` | **nada**, pluma; visor md in-process (preview) |
| imagen raster | `image/*` | **image-viewer** (ver), tullpu (editar pixel art) |
| audio | `audio/*` | **takiy**, media |
| video | `video/*` | **media** |
| Card (shared/card) | `application/x-tawasuyu-card` | **card-viewer**, agora (si es persona) |
| notebook | `application/x-pluma-notebook` | **pluma-notebook** |
| presentación | `application/x-pluma-deck` | **pluma-deck** |
| carta astral / efemérides | `application/x-cosmos-chart` | **cosmos** |
| dominio / zona DNS | `application/x-dominium` | **dominium** |
| simulación | `application/x-tinkuy` | **tinkuy** |
| diagrama | `application/x-chaka` | **chaka** |
| hoja de datos / CSV | `text/csv`, `application/x-nakui` | **nakui**, table-viewer |
| HTML / web | `text/html` | **puriy** |
| calendario | `text/calendar` | **raymi** |
| imagen wawa `.img` | `application/x-wawa-image` | **nahual** (montar), wawa-explorer |
| repo minga `.minga/` | `application/x-minga-repo` | **nahual** (montar), minga-explorer |
| fuente | `font/*` | **font-viewer**, tullpu |
| geo (GeoJSON/PMTiles) | `application/geo+json` | **map-viewer** |
| archivo (zip/tar) | `application/zip`… | **nahual** (montar), archive-viewer |
| proceso / unidad | (dominio) | **sandokan-monitor** |

Recíproco: cada app gana "Reveal in nahual" (abre nahual en el dir del archivo) — un helper en
`app-bus` (`reveal(path)` → spawnea `nahual-shell <dir>`). Así la integración es **bidireccional**:
nahual abre con cualquier app; cualquier app vuelve a nahual.

## 6. Mapa de archivos a tocar

```
nahual-source-core/src/lib.rs        Node+metadata, SourceMut, as_mut()        [F4.0]
nahual-source-core/src/posix.rs      impl SourceMut + stat                     [F4.0]
nahual-source-core/src/{wawa,nouser,minga}.rs  metadata opcional               [F4.0]
nahual-source-core/src/archive.rs    ArchiveSource (nuevo)                      [F4.6]
nahual-source-core/src/net.rs        NetSource (nuevo, opt-in)                  [F4.6]
llimphi/widgets/table|datatable      headers clicables, sort                   [F4.1]
nahual-shell-llimphi/src/main.rs     Model multi-pane, ViewMode, ops, openwith [F4.1-4.5]
nahual-shell-llimphi/src/ops.rs      OpQueue (nuevo)                           [F4.3]
nahual-shell-llimphi/src/state.rs    sled labels/places/formats (nuevo)        [F4.5]
nahual-shell-llimphi/src/viewer_registry.rs  cablear external_handler_for      [F4.4]
shared/app-bus/assets/apps/*.toml    Cards de las ~20 apps                     [F4.4]
shared/app-bus/src/lib.rs            reveal(path) helper                       [F4.4]
pata-llimphi/src/nouser.rs           consumir nahual-source-core::NouserSource [F4.7]
(retiros) chasqui-explorer-llimphi, wawa-explorer-llimphi                      [F4.7]
```

## 7. Riesgos / gotchas

- **`nahual-shell` es bin sin lib** → los pantallazos incluyen módulos por `#[path]` (ya se hace).
  Si el Model crece, considerar partir en lib + bin (facilita tests y examples). Ver
  `project_split_fat_apps`.
- **Hoja no-POSIX + open-with externo**: el tempfile pierde la extensión real → pasar el
  `mime_hint` o nombre original; los viewers son path-based (gotcha ya visto en BRAHMAN F3 paso B).
- **`SourceMut` en fuentes sintéticas**: NO implementarlo en wawa/minga (CAS inmutable). Gatear la
  UI, no fingir. Una Mónada "renombrar" no tiene semántica POSIX → deshabilitado.
- **Multi-agente**: commits con pathspec explícito por archivo (`feedback_git_multi_agent_pathspec`).
- **`cargo test -p` de fuentes con features**: correr con `--features nouser,minga` o no se compilan
  esos adapters.
- **Doble fuente de Mónadas (pata daemon vs nahual escaneo)**: F4.7 debe elegir UNA o factorizar; no
  dejar dos rutas divergentes (la razón de existir de este plan).

## 8. Métrica de éxito

1. **Una ventana** abre dir POSIX, `.img` wawa, Mónada nouser, repo minga, y `.zip` — todo montado
   por `Source`, navegado igual.
2. **Dual-pane + columnas ordenables + copiar/mover entre panes + batch rename** funcionando, con
   pantallazo headless por feature.
3. **"Abrir con"** lista apps reales de la suite y las lanza; doble-clic usa el default.
4. **Cero exploradores sueltos redundantes**: chasqui-explorer retirado, wawa-explorer degradado,
   pata sobre la fuente común. `cargo check --workspace` verde.
5. **Bidireccional**: cualquier app puede "Reveal in nahual".

## 9. Orden sugerido de ataque

`F4.0 → F4.1 → F4.4` primero (cimiento + cara Dopus + integración apps = el 80% del valor
percibido), luego `F4.2 → F4.3` (chasis pesado), después `F4.5 → F4.6 → F4.7` (power-user, fuentes
extra, saldo de deuda), y `F4.8` a demanda. Cada fase: bloque funcional + pantallazo + commit
`feat(nahual): …` + push.
