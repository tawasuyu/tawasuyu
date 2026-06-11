# BRAHMAN.md — la espina dorsal de la suite

> Manifiesto autoritativo. Snapshot: 2026-05-30. Cuando otra doc contradiga, esta gana sobre el tema "Brahman".
> Escrito en lenguaje denso para IA y para el autor.

## Qué fue la visión original

Brahman nació como un **broker universal** donde *todo* se comunicaría agnóstica y distribuidamente vía
**Cards** (módulos con flujos tipados): desde un subsistema de backend, hasta un widget de ventana, hasta una persona.
Un solo sistema nervioso para la suite entera.

## Qué pasó realmente: se bifurcó en TRES capas

La visión no fracasó ni triunfó — se partió en tres, y hay que **nombrarlas** porque el riesgo es tratarlas como
una sola cosa difusa. Dos de las tres están vivas; una está muerta.

```
┌───────────────────────────────────────────────────────────────────────────────┐
│ CAPA 1 — BrahmanNet (TRANSPORTE P2P).  ★ VIVA — lo más valioso de la suite.     │
│   shared/card/card-net : libp2p TCP+Noise+Yamux+Kademlia+identify+stream.       │
│   UN PeerId, UNA DHT, MÚLTIPLES protocolos sobre el mismo nodo:                 │
│     /minga/sync/1.0.0 · /agora/gossip/1.0.0 · /brahman/handshake/1.0.0          │
│   COMPARTIDA POR: minga + agora + ayni (open_with_node / sharing(net)).         │
│   PRIMITIVA UNIFICADORA: DhtKey = [kind_tag(1)] ++ [blake3(32)]                 │
│     RecordKind { Code(minga), Card(discovery), Persona(agora), Service(futuro) }│
│   => "todo direccionable distribuidamente en un namespace común" YA ES REAL.    │
├───────────────────────────────────────────────────────────────────────────────┤
│ CAPA 2 — Broker de Cards (MATCHING TIPADO local).  ◑ VIVA pero SUB-ADOPTADA.    │
│   chasqui-broker (matching Exact/Structural) instanciado en arje-zero (PID 1).  │
│   Infra: card-handshake (Hello→HelloAck/ULID) · card-sidecar · card-admin.      │
│   CONSUMIDORES REALES (~10 crates, vía sidecar/handshake):                      │
│     chasqui-{core,nous-mock,nous-real,explorer,broker-explorer} ·               │
│     cosmos-card · nakui-core · shuma-daemon · card-discovery(en minga).         │
│   card-discovery lo usan nahual-shell y agora-app.                              │
│   ALCANCE REAL: se detuvo en DAEMONS DE BACKEND. No llegó a widgets ni a gente. │
├───────────────────────────────────────────────────────────────────────────────┤
│ CAPA 3 — Módulos agnósticos vía WIT/WASM.  ✗ MUERTA → RELEGADA (Fase 1, 26-05-30)│
│   shared/card/card-wit existe; NO hay un solo archivo .wit en el workspace;     │
│   nada lo pide en build real (sólo examples + dev-dep de card-sidecar).         │
│   El "WASM agnóstico por interfaz" nunca se ejecutó.                            │
│   DECISIÓN: relegado, no borrado — documentado como DORMIDO (ver su README).    │
│   CONTRATO AGNÓSTICO REAL Y VIGENTE: shared/card (formato Card) + handshake      │
│   nativo Rust (card-handshake) + namespacing de DhtKey. WIT fue aspiracional.   │
│   NOTA: el tipo WitInterface vive en card-core y SÍ lo usa el broker para        │
│   matching estructural — eso es metadata viva; lo dormido es el PARSER de .wit. │
└───────────────────────────────────────────────────────────────────────────────┘
```

## Las dos piezas que faltan de la visión original

La visión decía "hasta widgets de ventana, hasta gente". Esas dos piezas **no se construyeron, pero están a un paso**
porque cada una ya reinventó localmente lo que la Capa 2 hace:

```
WIDGETS ↔ CARDS:
  nahual/viewer_registry::pick despacha visores por (lens, mime) pero HARDCODED in-process.
  El norte declarado de nahual es un "AppBus" donde los visores se registran por (lens, mime, priority).
  Eso ES una Card. Conectar nahual al broker realiza "hasta widgets de ventana".

GENTE ↔ CARDS:
  DhtKey::Persona YA está reservado en la DHT (Capa 1).
  agora descubre personas por su propio gossip, no por la espina.
  Rutear ese discovery por DhtKey::Persona realiza "hasta gente".
```

## Sobre la proliferación de "exploradores" (no es redundancia, es deuda conceptual)

No se pisan en función (modelos de datos distintos), pero crecieron cada uno por su lado:

```
nouser (chasqui-core)      clusteriza archivos POSIX en Mónadas semánticas (embeddings)
nahual-file-explorer       navega filesystem POSIX vivo (state machine UI)
wawa-explorer              lee objetos content-addressed de una imagen .img (DAG, BLAKE3)
minga-explorer             dashboard de peers/contenido/tráfico P2P
chasqui-explorer(+broker)  debuggers del broker
nakui / agora-app          navegadores de dominio
```

NO fusionar (datos incompatibles). La cura es una **espina de discovery común**: que cada explorador sea
productor/consumidor de Cards, y que **nahual-shell sea el FRONT universal** que abre una raíz de minga, un objeto de
wawa, una Mónada de nouser o un archivo POSIX — porque cada uno registra un visor-Card.

## Plan para ORDENAR (terminar de enchufar, no reconstruir)

```
FASE 0  Nombrar la realidad.  → ESTE DOCUMENTO.                                          [no destructiva] ✓
FASE 1  Decidir sobre WIT/WASM (Capa 3 muerta).  ✓ HECHA (2026-05-30): card-wit RELEGADO a
        "dormido" (doc en lib.rs + README propio); shared/card + handshake + DhtKey declarados
        contrato agnóstico real. No se borró (parser funcional, reversible).   [decisión cerrada]
FASE 2a nahual viewer_registry → Cards: visores se registran por (lens,mime,priority),
        el shell despacha vía broker.  REALIZA "widgets hablan por Brahman".            [refactor medio]
        ◑ PASO 1 HECHO (2026-05-30): viewer_registry ahora es DATA-DRIVEN — tabla de
          ViewerCard{kind,lenses,mime_prefixes,mime_exact,priority:card_core::Priority};
          pick() rankea por especificidad+Priority (mismo modelo que el broker). 17 tests verde.
          Agregar visor = agregar fila. registry() es la costura: hoy estática, mañana del broker.
        ✓ PASO 2 HECHO (2026-05-31): registry() ya NO es estática — se ensambla en runtime
          (OnceLock) como `builtin_registry() + discover_viewer_cards()`. Las descubiertas son
          `card_core::Card`s reales (JSON/TOML) leídas de `$NAHUAL_VIEWERS_DIR`
          (def. ~/.config/nahual/viewers.d), MISMO formato que card-discovery escanea y el
          broker anuncia. Extensiones de la Card: `nahual.viewer_kind` (→ ViewerKind),
          `nahual.mime_exact/_prefixes`; lens del `presentation_hint`; priority de la Card.
          Una Card que EXTIENDE el ruteo de un visor montado (p.ej. PSD→image) funciona
          end-to-end (reusa el constructor in-process, sin IPC). Cards con viewer_kind no
          montable se ignoran (visores out-of-process, pendientes del render-IPC). 24 tests
          verde + ejemplo en `viewers.d.example/`. discover_viewer_cards() es la costura final:
          cambiar "directorio en disco" por "broker" no toca el ranking.
FASE 2b agora personas → discovery por DhtKey::Persona.  REALIZA "gente entra a la espina". [refactor medio]
        ✓ HECHA (2026-05-31). Dos movimientos:
        (1) DhtKey/RecordKind — la "primitiva unificadora"— SE MUDÓ de minga-dht a
            `shared/card/card-net` (Capa 1, su lugar real: namespace COMÚN, no de un
            dominio). minga-dht la re-exporta (cero churn en minga-p2p/card-discovery).
        (2) AgoraNet (agora-net-brahman) ganó `anunciar_persona(&Identity)` /
            `anunciar_mis_personas()` / `dejar_de_anunciar_persona(&IdentityId)` /
            `descubrir_proveedores_de_persona(&IdentityId)->Vec<PeerId>`, sobre la MISMA
            BrahmanNet compartida. Clave = `DhtKey::for_hash(Persona, IdentityId)` =
            `[0x03]++blake3(pubkey)` (IdentityId YA es blake3(pubkey), no re-hashea).
            Gente entra a la espina con la misma primitiva que código (minga) y Cards.
            Test E2E real sobre el DHT (anunciar_y_descubrir_persona_via_dht) + example
            convergencia_minga extendido (un nodo, tres namespaces). Mismo tier que
            anunciar_gossip: primitiva lista, espera el daemon que también la llame.
FASE 3  Espina única de exploradores: nouser/nahual/minga/wawa-explorer como Cards;
        nahual-shell = front universal.  VISIÓN ORIGINAL REALIZADA.                     [visión realizada]
        ◑ PASO A HECHO (2026-05-31): crate `02_ruway/nahual/nahual-source-core` —
          trait `Source` agnóstico (label/root/children/read) + `Node{id,name,is_container}`
          object-safe. DOS adapters reales detrás de la misma interfaz: `PosixSource`
          (filesystem vivo) y `WawaImgSource` (objetos content-addressed de un `.img`,
          navega el DAG BLAKE3 por hash vía wawa-explorer-core; puro local). 9 tests
          contra backends REALES (un .img sintético + un tmp POSIX). El shell todavía no
          lo consume — es la espina, no el wiring.
        ✓ PASO B HECHO (2026-05-31): el shell consume `Source`. `nahual-source-core` ganó
          `Navigator` (estado de navegación genérico sobre `Box<dyn Source>`: pila/selección/
          scroll, gemelo agnóstico de FileExplorerState; 2 tests sobre POSIX). nahual-shell:
          `Model.mounted: Option<Navigator>`. Al abrir un archivo se intenta `WawaImgSource::abrir`
          (chequeo de magic barato, content-based) — si es imagen wawa se MONTA y el panel
          izquierdo desciende su DAG; cualquier otra cosa cae al open-with de siempre. Subir
          desde la raíz de la fuente la desmonta (vuelve a POSIX). Puente para hojas no-POSIX:
          `nav.read(id)` → tempfile → `load_for` (los visores siguen path-based; el tempdir
          vive en el Model mientras el visor streamea). Header muestra label+breadcrumb de la
          fuente. cargo check del shell+core verde (workspace roto aparte por WIP de khipu de
          otro agente). Limitación: extensión perdida en el tempfile ⇒ el demuxer de video cae
          a AV1 crudo; el discernimiento es por contenido así que el visor igual se elige bien.
        ✓ PASO C HECHO (2026-05-31): `NouserSource` (feature opt-in `nouser`):
          Mónadas semánticas de chasqui-core (scan→cluster, pseudo-embeddings deterministas,
          sin daemon). Tercera FORMA de árbol —clusters que no existen en disco— detrás del
          mismo trait. Gated para no arrastrar chasqui-core (sled/walkdir).
        ✓ PASO D HECHO (2026-05-31): `MingaSource` (feature opt-in `minga`): el grafo CAS de
          AST de un repo minga (`.minga/` sled vía `PersistentRepo`). CUARTA forma —DAG de
          `StoredNode{kind,leaf_text,children:[hash]}` etiquetado por `kind`—: raíz lista
          todos los nodos, descender muestra hijos del AST, hoja lee su token (`leaf_text`).
          14 tests con la feature (mete un AST real, lo lee de vuelta).
        ➡ ESPINA COMPLETA: los cuatro mundos del BRAHMAN.md (POSIX · wawa · nouser · minga)
          caben en un solo trait `Source` — jerarquía física · DAG de contenido · clusters
          semánticos · DAG de AST.
        ✓ PASO E HECHO (2026-05-31): FRONT cablea los cuatro. El shell habilita las features
          `nouser,minga` de nahual-source-core y monta por demanda: abrir un archivo con
          magic wawa auto-monta `WawaImgSource` (Paso B); `m` monta el dir objetivo (subdir
          seleccionado o cwd) como `NouserSource` (Mónadas, sólo lee); `g` lo monta como
          `MingaSource` SI `parece_repo_minga` (chequea artefactos sled `conf`/`db` SIN abrir
          — abrir crearía sled en un dir ajeno; guard anti-efecto-secundario verificado:
          sled deja `["blobs","conf","db"]`). ⌫ desde la raíz de la fuente desmonta a POSIX.
          Header POSIX muestra la pista de atajos. Front universal de los 4 mundos: realizado.
```

## Fase 4 — front universal nivel Directory Opus (plan aparte)

La Fase 3 dejó la **espina de datos completa** (un trait `Source`, cuatro mundos). El paso siguiente
—volver a `nahual-shell` un file manager pleno (dual-pane, columnas, operaciones, batch rename),
cablear el AppBus vivo ("abrir con" hacia las ~30 apps), y **absorber/retirar los exploradores
sueltos**— está planificado en **`UNIFICACION.md`** (2026-06-11). Ese doc es autoritativo sobre la
Fase 4; este sobre las Fases 0–3.

## Tesis de cierre

No hay un sistema fragmentado. Hay un **transporte unificado vivo (Capa 1)** con **dos clientes desconectados**
(widgets y gente) que **ya saben cómo conectarse**. Brahman no se rescata desde cero: se termina de enchufar.

## Mapa de rutas

```
shared/card/card-net        Capa 1 — BrahmanNet (libp2p)
shared/card/card-wit        Capa 3 — parser WIT (muerto)
shared/card                 contrato agnóstico real (formato Card)
02_ruway/chasqui/chasqui-broker      Capa 2 — broker de matching tipado
02_ruway/chasqui/card-handshake      Capa 2 — handshake Init↔módulo
02_ruway/chasqui/card-sidecar        Capa 2 — sidecar reusable
02_ruway/chasqui/card-admin          Capa 2 — snapshot del broker
03_ukupacha/minga/card-discovery     widget de descubrimiento de Cards (nexo UI↔broker)
03_ukupacha/arje/init/arje-zero      PID 1 — instancia el broker + levanta BrahmanNet
02_ruway/nahual/nahual-shell-llimphi viewer_registry hardcoded (candidato Fase 2a)
03_ukupacha/agora/agora-net-brahman  gossip sobre BrahmanNet compartida
```
