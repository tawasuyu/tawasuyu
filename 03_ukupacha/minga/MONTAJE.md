# Montar un nodo minga del monorepo

> Runbook para servir el repositorio tawasuyu como nodo minga y para que
> otros nodos lo sincronicen. Minga es un **VCS semántico P2P**: versiona el
> **AST** del código (no líneas, no blobs), direccionado por contenido
> (α-hash + BLAKE3) y firmado por identidad (DID Ed25519).

## Qué versiona (y qué no)

Minga ingiere el **AST de 5 lenguajes** — Rust, Python, TypeScript,
JavaScript, Go — vía tree-sitter. Del monorepo, eso son **2.111 archivos de
código** (los `.rs` y demás), **no** la documentación (`.md`), los
`Cargo.toml`, los assets (`.png/.webp`), `Cargo.lock` ni los scripts. El
`git clone` canónico del repo completo sigue siendo
[git.tawasuyu.net](https://git.tawasuyu.net/tawasuyu/tawasuyu); este nodo es
el **espejo semántico soberano** del código, no un reemplazo de Gitea (el
plan para que minga reemplace git por completo está en
[`shared/`/visión](#hacia-el-reemplazo-de-git)).

## Estado y advertencia de escala

Ingerir el monorepo genera **~1,44 millones de nodos AST** (2.067 raíces
α-hash, ~733 MB en sled). El **almacenamiento local funciona** sin problema;
lo que **no escala todavía** es el **sync P2P masivo**: `MingaPeer` carga el
grafo a RAM para sincronizar, y 1,44M nodos es justo el caso que activa el
refactor a `NodeStore` (sled directo, trigger histórico: >100k nodos). Hasta
ese refactor:

- **Servir el nodo (`listen`) y consultarlo localmente**: ✅ funciona.
- **Sync remoto completo del monorepo**: pesado (carga ~733 MB a RAM en el
  par). Recomendado sincronizar **subconjuntos** (un dominio) o esperar
  `NodeStore` para el sync masivo.

## Requisitos

```sh
cargo build -p minga-cli --release          # binario: target/release/minga
export MINGA_PASSPHRASE="<tu-passphrase>"    # operación no-interactiva (sin esto, pide TTY)
export TMPDIR=/mnt/vvv/tmp                    # sled fuera de la raíz (que suele estar apretada)
```

`MINGA_PASSPHRASE` es el equivalente al credential-helper de git: cifra el
keypair Ed25519 del nodo. Con la env seteada, ningún comando pide terminal —
es lo que permite correr el daemon como servicio.

## Montaje del nodo

```sh
BIN=/mnt/vvv/tawasuyu/target/release/minga
REPO=/mnt/vvv/minga/tawasuyu-nodo            # en disco persistente, NO en /tmp

# 1. inicializar identidad + almacén
"$BIN" --repo "$REPO" init                   # imprime el DID del nodo

# 2. ingerir el código del monorepo (AST)
"$BIN" --repo "$REPO" ingest-dir --recursive /mnt/vvv/tawasuyu

# 3. verificar
"$BIN" --repo "$REPO" status                 # DID, raíces α, nodos, atestaciones

# 4. servir a peers (daemon libp2p)
"$BIN" --repo "$REPO" listen /ip4/0.0.0.0/tcp/4001   # imprime el PeerId
```

### Identidad del nodo montado (2026-06-10)

- **DID**: `did:key:d9d8c18a672882ed2904c6528f3d538857e78abe1e1838405f7f5dbfb752ec98`
- **PeerId**: `12D3KooWLrivQGmcbbmndoSgZZFwGdTuu7sDPCRbRmikJ2NaWjGW`
- **Multiaddr** (LAN/local): `/ip4/<IP-del-host>/tcp/4001/p2p/12D3KooWLrivQGmcbbmndoSgZZFwGdTuu7sDPCRbRmikJ2NaWjGW`
- **Estado**: escuchando en `0.0.0.0:4001`, 2.067 raíces anunciadas en el DHT, 1.442.833 nodos en el almacén.

> Este DID/PeerId corresponden a un nodo **de demostración** (passphrase
> conocida). Para un nodo soberano de producción, re-inicializá con tu propia
> `MINGA_PASSPHRASE` — la identidad cambia y es la que otros peers anclan.

### Como servicio permanente (OpenRC)

El daemon `listen` muere al cerrar la sesión. Para que persista —igual que
el runner de CI— un servicio OpenRC (requiere la env del passphrase en el
entorno del servicio):

```sh
# /etc/init.d/minga-nodo  (command_user sergio, env MINGA_PASSPHRASE + TMPDIR)
sudo rc-update add minga-nodo default && sudo rc-service minga-nodo start
```

(El passphrase en el archivo de servicio es el trade-off de un daemon
desatendido; alternativa más segura: keyfile con permisos 600 leído por un
wrapper.)

## Acceder desde otro minga

Desde cualquier otro nodo minga, con el multiaddr de arriba:

```sh
export MINGA_PASSPHRASE="<passphrase-del-nodo-local>"
minga --repo ~/mi-repo init                  # si es la primera vez
minga --repo ~/mi-repo sync /ip4/<IP>/tcp/4001/p2p/<PeerId>
```

El `sync` jala el cono de nodos del nodo remoto. La identidad del repo
servido se verifica por su **DID**. NAT traversal: minga hereda
relay + DCUtR + AutoNAT de `card-net`, así que funciona aun sin IP pública
directa (vía relay).

Una vez sincronizado, los comandos de historia operan localmente:

```sh
minga --repo ~/mi-repo log <archivo>         # historia semántica
minga --repo ~/mi-repo diff <a> <b>          # diff de AST, no de líneas
minga --repo ~/mi-repo blame <archivo>       # autoría por nodo
```

## Hacia el reemplazo de git

Este nodo es el primer paso del dual-track: git sigue canónico, minga
acumula el historial semántico del código en paralelo. Las fases para que
minga reemplace git por completo (y para versionar por iteración como hoy se
hace con git):

1. **Passphrase por env** — ✅ hecho (`MINGA_PASSPHRASE`), desbloquea daemon, CI y automatización.
2. **Modo blob** — ingerir archivos no-código (`.md`, `.toml`, assets) como blobs BLAKE3, para cobertura total del árbol.
3. **Verbo `snapshot -m`** — unidad de historia con mensaje + padre, equivalente al commit.
4. **`NodeStore`** — sync P2P sobre sled directo (sin cargar a RAM); requisito para servir el monorepo completo a escala.
5. **Inversión** — minga pasa a canónico; git queda como espejo de export.
