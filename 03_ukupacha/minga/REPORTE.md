# minga — reporte técnico para IA

> Estado: **2026-05-27** · rama `main` · compila limpio (`cargo build -p minga-cli -p minga-explorer-llimphi -p shuma-module-minga`).
> Audiencia: sesión futura de Claude u otra IA que retome el VCS semántico.

---

## 1. Mapa actualizado

```
03_ukupacha/minga/
├── minga-core            ← AST + CAS + MST + atestaciones + α-hashing per-language
├── minga-store           ← sled: nodes, attestations, mst, roots (NUEVO), timestamps (NUEVO)
├── minga-dht             ← DhtKey typed; DhtKey::for_hash NUEVO (sin re-blake3)
├── minga-p2p             ← MingaPeer (libp2p) + ingest_with_dialect NUEVO + DhtKey en announce/find
├── minga-vfs             ← FUSE + render_source (NUEVO: Python indent-aware)
├── minga-cli             ← init, status, ingest, log NUEVO, show NUEVO, listen, sync (DHT NUEVO), watch (Remove NUEVO), mount
└── minga-explorer-llimphi ← dashboard + watcher wawa-config NUEVO
```

## 2. Cambios de este sprint

### Cableado α-hash al ingest (#1) — alto impacto
- `PersistentRepo` ahora abre **5 trees**: `nodes`, `attestations`, `mst`, `roots`, `attestation_timestamps`.
- Nuevo `SledRootsStore`: `α_hash → (struct_hash, dialect)`. Indirección que separa la **identidad del archivo** (α-hash, estable bajo renombrado) del **CAS del grafo** (struct-hash).
- `cmd_ingest`/`cmd_watch` ahora computan `α = hash_alpha_with(dialect, &node)`, lo registran como raíz del MST, lo firman con la atestación, y guardan `α → struct` en `roots`.
- `IngestResult` expone tanto `alpha` como `struct_hash` y `dialect`.
- `RepoSource::get` resuelve transparentemente: si `hash` es α (root), redirige al struct; si no, lo busca directo en `nodes`. Esto preserva la navegación `cas/<hash>` para nodos internos.

### DHT typed (#2)
- `minga-dht::DhtKey::for_hash(kind, [u8;32])` — nuevo constructor que **no re-blake3-ea** un hash existente.
- `MingaPeer::announce_provider`/`find_providers` ahora envuelven el `ContentHash` en `DhtKey::for_hash(RecordKind::Code, ...)`. Comparte la malla Kademlia con cards/personas sin colisión.

### CLI `minga log` y `minga show` (#3)
- `minga log [path]`: lista atestaciones ordenadas por timestamp local (descendente). Si pasás `path`, marca con `*` la entrada cuyo α-hash coincide con el contenido actual del archivo. Salida: `* YYYY-MM-DD HH:MM  <α-hash>  [dialect]  by <DID>`.
- `minga show <hash> [--sexp]`: pinta la fuente reconstruida (forma canónica) del nodo. Acepta α-hashes (raíces) y hashes estructurales del grafo CAS. Con `--sexp` devuelve el árbol literal.
- `SledTimestampStore`: timestamps locales de cuándo se observó cada atestación. **No** se transmite por wire — es metadata propia del peer.

### `watch` con remove tracking (#4)
- `cmd_watch` mantiene un `HashMap<PathBuf, ContentHash>` en memoria; el `initial_scan` lo popula con todos los archivos soportados.
- En `EventKind::Remove(_)` retira el α-hash del MST y de `roots` (vía nuevos métodos `SledMstStore::remove` y `SledRootsStore::remove`).
- Los **nodos del grafo CAS NO se eliminan** — pueden estar compartidos con otras raíces. Las atestaciones tampoco (siguen siendo prueba histórica).

### DHT lookup en sync (#6)
- `cmd_sync <target>` detecta si `target` es un hex de 64 caracteres (α-hash) y, en ese caso, hace `peer.find_providers(hash)` en el DHT. Itera los providers retornados intentando `sync_with` hasta éxito o deadline.
- Nuevo error `CliError::NoProvidersForHash` cuando el DHT no devuelve nadie. Necesita al menos un peer bootstrap conocido (`add_dht_peer`).

### Shebang detection (#7)
- `minga_core::parse::detect_by_shebang(source)`: reconoce `python*`, `node`, `deno`, `bun`, `tsx`, `ts-node`, `go`, `rustc`. Override por `--ext=ts` para deno.
- CLI: `detect_dialect` ahora prueba primero por extensión, después por shebang.
- `is_supported_source` (usado por `watch`) también consulta shebang — scripts sin extensión (`bin/foo`, `tool`) ahora se versionan.

### Python pretty-printer indent-aware (#9)
- `render_source` detecta el root kind `module` y delega a `render_python`.
- Recorre el AST de tree-sitter Python reconociendo statements compuestos (`function_definition`, `class_definition`, `if_statement`, `for_statement`, `while_statement`, `with_statement`, `try_statement`, `match_statement` + variantes async/decorated).
- Para cada compound: separa el header (todo lo que no es `block`/cláusulas) y emite `header:` + `block` con `indent + 1`. Cláusulas anidadas (`elif_clause`, `else_clause`, `except_clause`, `finally_clause`, `case_clause`) se recursan al mismo nivel.
- Tests verifican `def`/`return`, `if`/`else`, y `class` con método (indentación 4/8).

### Wawa-config watcher en explorer (#10)
- `minga-explorer-llimphi` ahora depende de `wawa-config`.
- `init` carga `WawaConfig::load()`, mappea `theme_variant` vía `canonical_theme_name` + `Theme::by_name`, aplica `accent_rgb`, llama `rimay_localize::set_locale(&cfg.lang)`.
- `ConfigWatcher::spawn` con closure que `handle.dispatch(Msg::WawaChanged(cfg))` — reactiva theme/lang sin reinicio.

### `shuma-module-minga` (#8) — feature nueva
- Tab del shell shuma que muestra el repo Minga del cwd.
- Counts (`raíces / nodos / atestaciones / mst`) + lista de raíces recientes con su α-hash corto y dialect.
- Shortcut "Refresh" + monitor "minga · raíces" en el panel derecho.
- El chasis `shuma-shell-llimphi` registra el nuevo `Kind::Minga`, ramas en `update`/`view` (Main + DrawerTab + contributions), y el handler de `minga.refresh` que lanza `load_snapshot` en un thread.

## 3. Diferido

### MingaPeer genérico sobre NodeStore (#5)
**No implementado** — requiere generizar `PeerState`, `SyncSession`, `snapshot`, y `merge_into_state` sobre un trait `NodeStore` (que ya existe en `minga-core::store`). Toca el wire protocol indirectamente porque las sesiones de sync clonan el store entero.

Razón para diferir: alto costo de refactor + tests, beneficio sólo se manifiesta en repos grandes que minga aún no tiene. Cuando el primer repo supere los 100k nodos, retomar.

## 4. Comandos útiles

```bash
# Init + ingest + log + show
minga -r ./.minga init
minga -r ./.minga ingest src/main.rs
minga -r ./.minga log src/main.rs    # marca con * el α actual
minga -r ./.minga show <alpha_hex>

# Watch (autoingest + autoremove)
minga -r ./.minga watch ./src

# Sync por DHT (necesita peer bootstrap)
minga -r ./.minga sync <alpha_hex>   # busca providers
minga -r ./.minga sync <multiaddr>   # conexión directa

# Mount FUSE
mkdir mnt && minga -r ./.minga mount mnt
ls mnt/roots/         # un archivo por α-hash
cat mnt/roots/<α>     # fuente reconstruida (Python indent-aware ahora)

# Explorer Llimphi (con theme reactivo via wawa-config)
MINGA_REPO=./.minga cargo run -p minga-explorer-llimphi
```

## 5. Diseño preservado

1. **Sync protocol intacto.** Los α-hashes del MST viajan como `ContentHash`es por el wire (32 bytes); el receptor no necesita re-verificar α — confía en la atestación firmada. La indirección α→struct es local a cada peer.
2. **`MemStore` sigue siendo el medio de sync** entre peers; `MingaPeer::open` carga todo a RAM como antes. La generización a `S: NodeStore` queda como item #5.
3. **Tree `roots` separado del MST.** El MST contiene sólo los α-hashes (claves), igual que antes — la nueva indirección `roots` es independiente. Esto preserva todos los tests del protocolo.
4. **`SledTimestampStore` es local.** Dos peers que ven la misma atestación tendrán timestamps distintos (cuando llegó a cada uno) — esto es deliberado: `minga log` es una vista local del historial.

## 6. Sub-sprint posterior (5 items adicionales completados)

| # | Tarea | Estado |
|---|---|---|
| 11 | **`minga diff`** entre dos hashes (LCS vía `similar` crate) | hecho — el test `rename_local_var_keeps_same_alpha_hash` valida que α se manifiesta end-to-end |
| 12 | **`minga retire`** — tombstone firmado (`Retraction` con `RETRACTION_DOMAIN` prefix; `SledRetractionStore` paralelo a atestaciones; quita del MST y `roots` pero conserva la atestación original como prueba histórica) | hecho |
| 13 | **`minga verify`** — `verify_root_alpha(node, claimed) -> Option<Dialect>` que prueba cada dialecto; el CLI reporta consistencia + drift con dialect registrado | hecho (la re-verificación al recibir-wire requiere modificar el protocolo de sync, documentado abajo) |
| 14 | **Click en raíz del módulo shuma-module-minga**: dispara `SelectRoot(hash)` → el chasis spawnea `load_root_source` en thread → resultado vía `SourceLoaded` → panel inferior con `render_source` | hecho — race-protect: si llega un click nuevo mientras carga el anterior, el resultado viejo se descarta |
| 15 | **Detección de dialect por contenido**: marcadores textuales por línea (`def`/`fn`/`func`/`function`/`interface`) + tie-break por ratio de nodos ERROR. `detect_dialect` ahora prueba ext → shebang → contenido | hecho |

### #13 (verify) — alcance honesto

La re-verificación se ofrece como primitiva (`alpha::verify_root_alpha`) y como subcomando (`minga verify <hash>`). Verifica **localmente** que una raíz del repo es consistente bajo algún dialect; útil tras sync con peers no-confiables.

**No** intercepta automáticamente en el path de sync porque el wire actual no transmite dialect ni el binding α→struct. Para integrarlo ahí hay que extender `minga-p2p::session::Message` con una variante `RootDeclaration { alpha, struct_hash, dialect }`; queda para una fase futura.

## 7. Próximos pasos abiertos

| # | Tarea | Prioridad |
|---|---|---|
| A | Cachear `MingaPeer` con backend sled directo (item #5 deferido) | media |
| B | Extender wire de sync con `RootDeclaration { alpha, struct, dialect }` + re-verificar α automáticamente al recibir | media (seguridad) |
| C | `minga prune-cas` — recolector de basura del grafo CAS (nodos ya no referenciados por ninguna raíz) | media |
| D | Sync de `Retraction`s — wire actual sólo transmite `Attestation`s | media |
| E | `minga show --diff-against <other>` — combinar show+diff en un sólo comando | baja UX |
| F | Exportar `roots` como API REST/JSON desde un daemon minga | baja |
| G | `shuma-module-minga`: shortcut "Verify" que corre `cmd_verify_root` sobre cada raíz visible y marca consistentes/inconsistentes | media UX |

---

*Generado por Claude (Opus 4.7) — `2026-05-27`. 14/15 tareas completadas; #5 deferido (refactor invasivo, alto costo, beneficio sólo en repos grandes).*
