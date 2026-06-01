# ARQUITECTURA.md — chasqui

> Descripción técnico-arquitectónica densa, optimizada para consumo por IA.
> Snapshot: 2026-05-30. Fuente autoritativa cuando difiera con la prosa de los READMEs.

```yaml
DOMINIO: chasqui
CUADRANTE: 02_ruway (HACER)
NOMBRE: quechua "mensajero del camino del Inca"
ADVERTENCIA_SEMÁNTICA:
  El README/LEEME aspira a "broker de mensajería + bus tipado pub/sub".
  El CÓDIGO ACTUAL implementa otra cosa: NO hay transporte de mensajes app↔app en tiempo real.
  La aspiración pub/sub migró al dominio Ayni (chat P2P). Ver project_ayni_chat.md.
TESIS_REAL: dominio DUAL — (A) matcher determinista de tipos entre módulos + (B) inteligencia de datos semántica
TAMAÑO: 11 crates, ~8.7 KLoC. Brahman ≈4.3K · Nouser ≈4.4K
```

## Los dos subsistemas (no confundir)

```
SUBSISTEMA A — BRAHMAN CARD BROKER  (plano de TIPOS / control)
  Registry + matcher determinista de FLUJOS tipados entre Cards (módulos con interfaz WIT opcional).
  Responde "¿quién produce el flujo que este consumer necesita?". NO mueve datos: hace matching.
  Crates: chasqui-broker · card-handshake · card-sidecar · card-admin · chasqui-broker-explorer-llimphi

SUBSISTEMA B — NOUSER MONADS + EMBEDDINGS  (plano de DATOS / semántica)
  Escanea directorios → agrupa archivos en MÓNADAS (clusters semánticos) → enriquece con embeddings
  → permite búsqueda/atracción por centroide. Persiste en sled. UI consulta.
  Crates: chasqui-core · chasqui-card · chasqui-nous · chasqui-nous-mock · chasqui-nous-real · chasqui-explorer-llimphi

NEXO entre A y B:
  Los proveedores de embeddings (nous-mock/real) se REGISTRAN como Cards en Brahman.
  Un consumidor que pide "embed-request:json" es matcheado por el broker al proveedor correcto según CONTEXTO.
  => B usa A para descubrir QUÉ proveedor de embeddings está vivo y debe ganar.
```

## SUBSISTEMA A — Brahman Broker

### Tipos núcleo (`chasqui-broker/src/lib.rs`, 995 LOC, lib pura sin IO)

```rust
enum MatchStrategy { Exact, Structural, ExactThenStructural }
  // Exact      = TypeRef idéntico (interface+package+name)
  // Structural = mismo package+name, ignora interface (o mismo Primitive)
  // ExactThenStructural = prefiere exact, cae a structural

struct BrokeredCard {
  session: SessionId,        // ULID del handshake
  label: String, lifecycle: Lifecycle, priority: Priority,
  inputs: Vec<Flow>, outputs: Vec<Flow>,
  wit: Option<WitInterface>, // matching fino si el módulo es "consciente"
  priority_contexts: BTreeMap<String, ContextBias>,  // sesgos por contexto ("test"/"prod")
  kind: CardKind /*Ente|Data*/, data: Option<DataFacet>, service_socket: Option<PathBuf>,
}
struct Match { consumer: Endpoint, producer: Endpoint, ty: TypeRef, via: MatchStrategy, pinned: bool }
struct ContextBias { pin_to: Option<String>, priority_offset: i16 }
struct Broker { cards: BTreeMap<SessionId, BrokeredCard>, config: BrokerConfig }
```

### Algoritmo `find_producer_for(consumer, input_name)`

```
1. PIN: si el input (o priority_contexts[ctx].pin_to) fija un label → buscar productor con ese label (type-agnóstico).
2. TYPE-SEARCH: filtrar outputs de otras Cards por compatibilidad según strategy.
3. RANK: ordenar por effective_priority(producer) DESC, luego label ASC (tie-break determinista).
4. → Match { ... via, pinned }.
PROPIEDADES: determinista (mismas Cards + mismo contexto ⇒ mismo resultado); O(n·m); stateless en rutas.
```

### Handshake e infra (protocolo nativo Rust↔Rust, NO WIT/WASM)

```
card-handshake (2942 LOC): Hello{card} → HelloAck{session_id: ULID} sobre Unix socket.
  Frames length-prefixed + postcard. Sesión viva con Ping(~30s)/Farewell. Deriva TrustLevel (agnóstico vs WIT-consciente).
card-sidecar (565): thread tokio current_thread que mantiene la sesión. API await_provider(card,timeout), list_matches().
card-admin (225): Unix socket SEPARADO; emite StatusSnapshot JSON (sesiones+matches). Single-shot/conexión. bin `brahman-status`.
```

### INVARIANTES (A)

```
A-INV-1  El broker NO rutea datos; sólo computa matches de tipo bajo demanda. El data-plane lo abre cada módulo (service_socket).
A-INV-2  Matching determinista y reproducible: orden total por (priority efectiva, label).
A-INV-3  pin_to gana sobre type-search; priority_contexts[ctx] override estático. Contexto activo = BRAHMAN_BROKER_CONTEXT.
A-INV-4  El broker vive en memoria del Init. NO hay snapshot/recover al reboot (deuda → ver ASPIRA).
```

## SUBSISTEMA B — Nouser

### Mónada (`chasqui-card/src/lib.rs`, 709 LOC)

```rust
struct FileEntry { id: FileId/*ULID*/, path, content_hash: Option<[u8;32]>/*blake3*/, size, mtime_ms, extension }
enum Lens { Grid, Code, Gallery, Database, Markdown, Tree }   // cómo se visualiza el cluster
struct MonadManifest {
  schema_version: u16/*=1*/, id: MonadId/*ULID, ordenable por tiempo*/, lineage: Option<MonadId>,
  label, summary, centroid: Vec<f32>, centroid_model: Option<String>/*"chasqui-pseudo-32d"|"real-fastembed-384d"*/,
  path_hint: Option<String>/*dir padre canónico = identidad estable*/, keywords: Vec<String>,
  cardinality: u32, entropy: f32/*[0,1] cohesión de extensiones*/, dominant_lens: Lens,
  pins: BTreeSet<FileId>/*anclados, no migran*/, members: BTreeSet<FileId>,
  created_at_ms, updated_at_ms, extensions: BTreeMap<String, Value>/*forward-compat*/,
}
```

### Pipeline determinista (`chasqui-core`, 2215 LOC)

```
scanner.rs   recorre directorios → Vec<FileEntry>
cluster.rs   by_directory: agrupa por (dir padre + extensión dominante) → MonadManifest
db.rs        MonadDb: store en memoria + sled opcional
embed.rs     embed(file) → [f32;32] L2-normalizado, DETERMINISTA sin LLM:
               dims 0..8 blake3(extension) · 8..16 blake3(parent) · 16..24 blake3(stem) · 24..28 size(log) · 28..32 mtime(cíclico)
               EMBED_DIM=32, MODEL_ID="chasqui-pseudo-32d"
CLI:  nouser scan|show|json|daemon|attract
```

### Contrato Nous (`chasqui-nous`, 196 LOC) — proveedor de embeddings intercambiable

```
WIRE: JSON line-delimited sobre Unix socket, single-shot/conexión.
  EmbedRequest{ kind: EmbedFile|EmbedText|Ping, payload: Value }
  EmbedResponse{ embedding: Vec<f32>, model: String, elapsed_ms: u64 }
SOCKET: $XDG_RUNTIME_DIR/chasqui-nous-{provider}.sock  (override $NOUSER_NOUS_SOCKET)

mock (chasqui-nous-mock): chasqui_core::embed, 32d determinista; Card priority_contexts["test"]={+1}  → gana en test
real (chasqui-nous-real): fastembed+ONNX all-MiniLM-L6-v2, 384d; feature `embeddings`; cache sled;
                          Card priority_contexts["prod"]={+1}  → gana en prod
=> El broker Brahman elige mock vs real según BRAHMAN_BROKER_CONTEXT. Mismo flow embed-request:json / embed-result:json.
```

### INVARIANTES (B)

```
B-INV-1  embed() es puro y determinista (hash+metadata) ⇒ misma carpeta ⇒ mismas Mónadas y centroides en toda máquina.
B-INV-2  path_hint es la identidad estable de una Mónada (sobrevive a cambios de miembros).
B-INV-3  El proveedor de embeddings es opaco/intercambiable tras el contrato Nous; el core no sabe si es mock o LLM.
```

## UIs Llimphi

```
chasqui-broker-explorer-llimphi (599): probe de salud; poll 5s await_provider_blocking →
  estado Down | UpNoProvider | UpWithProvider(label,socket) + timeline de MatchEvent (últimos 50).
chasqui-explorer-llimphi (501): explorador de Mónadas; descubre el daemon nouser vía broker,
  consulta Mónadas, filtra por centroide (búsqueda semántica), muestra cardinality/entropy/lens.
```

## Relaciones inter-dominio

```
agora   : identidad Ed25519 — autores de mensajes en el futuro chat (vía Ayni), no en chasqui hoy.
minga   : transporte P2P libp2p (BrahmanNet) — chasqui aún NO lo usa; el broker es LOCAL/LAN (Unix socket).
akasha  : VFS content-addressed en wawa — no usado por chasqui hoy.
rimay   : chasqui-explorer-llimphi → rimay-localize para búsqueda semántica sobre centroides.
shuma   : chasqui-core → shuma-discern (clasificación determinista) — integración superficial, pendiente profundizar.
arje    : chasqui-nous-real podría persistir embeddings en CAS de wawa — experimental.
ayni    : HEREDA la aspiración pub/sub de chasqui. Ayni-sync usa trait Transporte (EnlaceMinga), NO el broker chasqui.
```

## Estado (2026-05-31)

### Hecho
- Subsistema A (Brahman): matching tipado determinista (Exact/Structural/ExactThenStructural) + context biases + handshake Unix-socket (card-handshake/sidecar) + observabilidad admin (`brahman-status`) + UI probe (`chasqui-broker-explorer-llimphi`).
- Subsistema B (Nouser): scanner + clustering `by_directory` + `MonadDb` (sled) + pseudo-embeddings 32d deterministas + real 384d ONNX gated por feature + contrato Nous (mock/real intercambiables) + UI explorer semántico + CLI `nouser`.
- Nexo A↔B: los proveedores de embeddings se registran como Cards; el broker elige mock vs real por `BRAHMAN_BROKER_CONTEXT`.
- Las dos UIs portadas a Llimphi (GPUI extinto) + menú principal/contextual (lotes 1 y 5).

### Pendiente
- Persistencia del broker: hoy vive en memoria del Init, sin snapshot/recover al reboot.
- Transporte remoto: Brahman es Unix-socket local; el matching de módulos remotos (card-net/libp2p) ya NO está bloqueado por NAT — `card-net` cablea relay+dcutr+autonat (verificado por el test `jalar_a_traves_de_un_relay` de khipu). Lo que falta es CABLEAR el discovery: el daemon Nouser no llama `announce_outputs()` al DHT ni hay `find_remote_providers(flow_type)` en `card-sidecar`.
- real-nous en producción: la feature `embeddings` arrastra ~200 MB de ONNX runtime (trade-off tamaño↔capacidad).
- Integración a fondo cross-dominio (rimay/shuma/arje) con nouser, aún superficial.
- La aspiración pub/sub original migró a Ayni (chat P2P); chasqui hoy NO transporta mensajes app↔app en tiempo real.

## Estado vs aspiración

```
DEUDA / ASPIRA_A:
  #1 PERSISTENCIA del broker: hoy vive en memoria del Init, sin snapshot/recover al reboot.
  #2 TRANSPORTE remoto: Brahman es Unix-socket local; falta CABLEAR discovery por DHT
     (announce_outputs en el daemon Nouser + find_remote_providers en card-sidecar).
  #3 NAT traversal: HECHO en card-net (relay+dcutr+autonat), heredado por minga/agora/
     chasqui/khipu; ya no bloquea el uso remoto P2P.
  #4 real-nous en producción: feature `embeddings` arrastra ~200MB de ONNX runtime — trade-off size↔capacidad.
  #5 COHERENCIA cross-dominio: rimay/shuma/arje aún no integrados a fondo con nouser.

NORTE_ARQUITECTÓNICO:
  Dos planos que convergen. PLANO DE TIPOS (Brahman): "decláme qué consumís y te encuentro quién lo produce,
  determinista y observable, sin acoplar módulos". PLANO DE DATOS (Nouser): "dame una carpeta y te devuelvo su
  estructura semántica latente (Mónadas) con embeddings intercambiables".
  El destino es que el broker deje de ser local (Unix socket) y, sobre transporte minga/libp2p, haga discovery
  tipado de pares REMOTOS — habilitando que Ayni (chat P2P) y otros descubran proveedores por tipo a través de la red,
  no por config. Chasqui hoy es una joya quieta: matching + búsqueda + discovery local, lista para crecer a P2P.
```

---

**Síntesis de una línea para otra IA:** chasqui es un dominio dual — (A) un **broker de tipos** Brahman que
hace matching determinista y observable de flujos entre módulos (Cards) sin mover datos, y (B) **Nouser**, un motor de
inteligencia de datos que agrupa archivos en Mónadas semánticas con embeddings intercambiables (mock 32d determinista /
real 384d ONNX) descubiertos a través de (A) — cuya aspiración pub/sub original migró a Ayni y cuyo norte es elevar el
discovery tipado local a discovery P2P remoto sobre transporte minga/libp2p.
