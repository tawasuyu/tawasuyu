# ARQUITECTURA.md — minga

> Descripción técnico-arquitectónica densa, optimizada para consumo por IA.
> Snapshot: 2026-05-30. Fuente autoritativa cuando difiera con la prosa de los READMEs.

```yaml
DOMINIO: minga
CUADRANTE: 03_ukupacha (RAÍZ)
NOMBRE: quechua "trabajo comunitario voluntario"
TESIS: VFS P2P content-addressed que versiona CÓDIGO COMO AST (no texto); la verdad es el BLAKE3 del contenido, no el path
PARADIGMA: VCS semántico distribuido + DHT + FUSE; sin servidor, sin autoridad; latencia comunitaria/doméstica, no CDN global
TAMAÑO: ~10 KLoC, 8 crates
ESTADO: octavo sprint cerrado (2026-05-29) — backlog completo salvo 1 item trigger-driven
```

## Idea-fuerza

```
IDENTIDAD = ESTRUCTURA, NO PATH:
  Cada archivo se parsea a SemanticNode (AST normalizado) y se direcciona por BLAKE3 de su estructura lógica.
  α-hashing per-lenguaje: dos archivos con misma estructura bajo renombrado de variables ligadas ⇒ mismo α-hash.
  => versionar SIGNIFICADO, no líneas. Renombrar una variable local no cambia la raíz α.

VERDAD CRIPTOGRÁFICA:
  Un peer NUNCA puede colar contenido falso bajo un hash legítimo: hash_components() es función pura,
  el receptor re-verifica hash_stored(stored)==hash antes de insertar. Sin confianza en el peer.
```

## Crates

```
minga-core   (3640) PURO sin IO: SemanticNode(AST) · ContentHash(BLAKE3) · Mst(Merkle Search Tree) ·
                    Attestation/Retraction(Ed25519) · Did · α-hashing per-dialect (Rust/Py/JS/TS/Go/…)
minga-store  (1077) sled 8-tree write-through: nodes(CAS) · attestations · mst · roots(α→struct+dialect) ·
                    path_history · alpha_paths · retractions · timestamps. "Same shape" que wawa-fs.
minga-dht     (223) discovery typed sobre Kademlia compartida: DhtKey = [kind_tag(1)]++[blake3(32)];
                    RecordKind{Code,Card,Persona,Service} evita colisión de namespaces en la MISMA DHT.
minga-p2p    (1504) MingaPeer (API alto nivel) + SyncSession (máquina de estados) + protocolo /minga/sync/1.0.0.
minga-vfs    (1018) VFS distribuido vía FUSE: path→DHT→lazy chunk fetch; cooperative fetch de peer cercano.
minga-cli    (2948) init/ingest/log/show/blame/history/roots/sign/verify/retire/prune/diff/watch/sync/mount/bundle/serve.
minga-explorer-llimphi (439) dashboard: peers · content local · tráfico; watcher reactivo a wawa-config.
card-discovery        (245) widget de descubrimiento de Cards Brahman; consumido por nahual-shell y agora-app.  ← NEXO BRAHMAN
```

## Transporte: `BrahmanNet` (re-export de `shared/card/card-net`)

```
STACK libp2p: TCP + Noise + Yamux + Kademlia(MemoryStore, modo Server) + identify + stream::Behaviour.
UN SOLO NODO, MÚLTIPLES PROTOCOLOS sobre el mismo PeerId:
  /brahman/handshake/1.0.0   (identidad remota — card-handshake)
  /minga/sync/1.0.0          (sync de código — minga)
  /agora/gossip/1.0.0        (web-of-trust — agora)
CONVERGENCIA: MingaPeer::open_with_node(Arc<LibP2pNode>) y AgoraNet::sharing(net) ADOPTAN el mismo nodo.
  La convergencia es a nivel de TRANSPORTE, no de protocolo wire. Demo: agora-net-brahman/examples/convergencia_minga.rs
```

## Protocolo de sync `/minga/sync/1.0.0` (anti-entropy simétrico request/response, NO gossip)

```
1. Challenge{nonce}×2          anti-replay, ambos peers
2. Hello{did, root_subtree_hash, sig}   sig sobre (peer_nonce||my_nonce||did||root); si roots iguales ⇒ done
3. ProbeReq{subtree_hash} / ProbeRes{NodeProbe{level,keys,child_hashes}}   descenso recursivo del MST, poda ramas idénticas
4. Fetch{hash} / Deliver{hash, StoredNode}   receptor VERIFICA hash_stored==hash antes de insertar (anti-malicia)
5. AttestPush{Vec<Attestation>}   propaga autoría tras Hello autenticado; cada firma Ed25519 verificada
6. RetractPush{Vec<Retraction>}   tombstones firmados con RETRACTION_DOMAIN (anti-replay)
7. RootDeclaration{Vec<RootDecl{α,struct_hash,dialect}>}   receptor RE-VERIFICA verify_root_alpha(node,α) contra contenido recibido
8. Done×2          sesión termina cuando ambos Done cruzan
CODIFICACIÓN: postcard. ROL: simétrico (sync_with activo / run_passive_accept pasivo).
```

## INVARIANTES

```
M-INV-1  Deliver se inserta sólo si hash_stored(stored)==hash. Contenido falso bajo hash legítimo = imposible.
M-INV-2  RootDeclaration se acepta sólo si verify_root_alpha(reconstruido, α) coincide. α-mappings maliciosos = rechazados.
M-INV-3  Atestaciones/Retracciones: firma Ed25519 verificada independientemente con la pubkey del Did.
M-INV-4  α-hash invariante bajo renombrado de variables ligadas (per-dialect). Convergencia estructural, no textual.
M-INV-5  write-through best-effort: si el IO a sled falla, la RAM sigue autoritativa; el próximo sync repopula.
```

## NAT / discovery — qué hay y qué falta

```
HAY:    TCP+Noise (auth en transporte) · Kademlia DHT · identify (auto-inyecta listen-addrs) · stream multiplexado.
        bootstrap automático en listen (sprint #4).
FALTA:  UPnP/IGD · hole-punching · relay nodes · mDNS local.
IMPLICACIÓN: hoy peers en misma LAN o con multiaddr explícito (add_dht_peer). Diseñado para latencia doméstica, no CDN global.
```

## Relaciones inter-dominio

```
agora   : COMPARTE el nodo BrahmanNet — un PeerId, una DHT, dos protocolos (sync + gossip). Convergencia de transporte.
wawa    : MISMO content-addressing BLAKE3; minga-store "same shape" que wawa-fs; chunks en $XDG_DATA_HOME/minga/chunks/.
card-*  : card-discovery (dentro de minga) consume el broker Brahman; card-net es la base de transporte compartida.
ayni    : ayni-minga usa card-net/BrahmanNet como impl del trait Transporte (chat P2P soberano).
chasqui : sin vínculo de código directo; el broker chasqui es local (Unix socket), minga es la capa P2P real.
akasha  : potencial indexación semántica de SemanticNode — no integrado.
```

## Estado vs aspiración

```
HECHO (8 sprints, 2026-05-29): ingest/sync/FUSE/CLI completo · α-hashing multi-dialect · blame/history/roots ·
  sign(vouching colaborativo) · retire(tombstone) · bundle v1+multi(zstd, magic MNGM/MNGZ) · serve(HTTP+Bearer auth) ·
  RootDeclaration con re-verificación α · DHT bootstrap automático · convergencia transporte con agora.

PENDIENTE ÚNICO (trigger-driven):
  #5/A  MingaPeer genérico sobre NodeStore (backend sled directo sin cargar todo a RAM).
        Bloqueado por costo de refactor (NodeStore::get→owned, cascada en SyncSession/tests).
        Trigger: primer repo real >100k nodos. Hoy sin caso ⇒ diferido con criterio.

ASPIRA_A (especulativo, no comprometido):
  NAT traversal global (UPnP/hole-punching/relay) · mDNS local · más dialects (C++, etc.) ·
  CRDT colaborativo (Yrs) para edición real-time (diseño aprobado, no implementado) ·
  búsqueda semántica vía akasha · reputación de provisión vía ayni.

NORTE_ARQUITECTÓNICO:
  minga es la CAPA P2P REAL de la suite (la única con libp2p vivo, DHT, FUSE y verificación criptográfica end-to-end).
  Versiona significado, no texto. Su destino es ser el sustrato de almacenamiento/sync distribuido SOBRE EL QUE corren
  agora (confianza), ayni (chat) y eventualmente wawa (releases) — todos compartiendo el mismo nodo BrahmanNet.
```

---

**Síntesis de una línea para otra IA:** minga es un VCS semántico P2P content-addressed que parsea código a AST
normalizado y lo direcciona por BLAKE3 estructural (con α-hashing invariante bajo renombrado), sincronizado entre peers
por un protocolo anti-entropy simétrico de 8 mensajes con verificación criptográfica de cada nodo, montable como FUSE y
descubrible por una Kademlia DHT typed — y es la **única capa P2P realmente viva** de la suite (libp2p `BrahmanNet`
compartido con agora y ayni), cuyo norte es ser el sustrato de almacenamiento distribuido de todo el ecosistema.
