# Plan — dotfiles, secretos y aislamiento de FS por contexto

Documento de diseño vivo para `pacha-dotfiles` y su extensión a **secretos** y
**aislamiento de directorios por pacha/app**. Fuente de verdad del rumbo; las
decisiones abiertas están al final.

## Tesis

Un contexto (`pacha`) no sólo conmuta config y apps: también fija **qué archivos
ve cada app y desde qué `$HOME`**, y **qué secretos** se le entregan — sin que
esos secretos toquen el disco si no se quiere. Todo se compone reusando lo que ya
existe:

- `shared/format` (Fase 66) = el modelo de objetos de git (`blob`/`árbol`/
  `symlink`/`+x`), content-addressed, `no_std`. El "git" del respaldo.
- `shared/pacha` = el conmutador por contexto (máquina de `Effect` pura + manager).
- `card-core::SomaSpec` = la **frontera física** de un Ente: `namespaces.mount`,
  `run_as` (uid distinto), cgroups, seccomp. Donde vive el aislamiento.
- `format/firma.rs` + agora = identidad Ed25519, base del cifrado.

`pacha-dotfiles` se generaliza de "versionar archivos en disco" a **un proveedor
de archivos/secretos con destinos de materialización enchufables**
(`disco | montaje-efímero`). La capa captura/store/historia NO cambia; sólo crece
el destino y gana una capa de cifrado en el store.

## Principios

1. **Secreto por defecto.** Todo blob se cifra en reposo a la identidad del
   usuario. "Compartir/publicar" es un verbo explícito (re-cifrar a destinatarios
   o exportar en claro), jamás el camino por omisión.
2. **El secreto puede no tocar el disco.** Descifrar en RAM y montar como tmpfs
   sólo dentro del mount namespace de los Cards elegidos. Se evapora con el
   namespace.
3. **El aislamiento vive en `SomaSpec`, no en pacha.** pacha sólo *referencia* el
   perfil de FS al encarnar las apps de su receta. La frontera es física
   (namespace), no una tabla de permisos — coherente con la doctrina de wawa.
4. **Certificación por texto.** Cada fase se verifica con tests headless: hashes,
   conteos, "este proceso ve X y el otro no", `ffprobe`-style stats. El render es
   último recurso (Regla 8 del repo).

## Estado actual (Fase 0 — HECHA, 2 commits en main)

- `pacha-dotfiles`: `StoreObjetos` (objetos por hash `aa/bbbb`), `capturar`/
  `materializar` ($HOME⇄grafo, preserva symlink y +x), `Instantanea` (commit
  raíz+padre = DAG), `commitear`/`historial`, `ConjuntoDotfiles`/`RutaGestionada`/
  `ModoGestion{Fijado,Rastreado}`. 9 tests.
- Integración pacha: `DotfileRef` + campo `dotfiles` en `Pacha`; efectos
  `MaterializarDotfiles`(entrar)/`CapturarDotfiles`(salir); `dotfile_pins` en
  RuntimeState (pin que avanza por recaptura, gana a la instantánea clavada).
  `LinuxSurfaces::with_dotfiles(DotfilesCtx)` ejecuta lo real. 4 tests.
- `cargo check --workspace` verde.

## Fase 1 — Aislamiento de FS + destino efímero (mecanismo HECHO; falta wiring pacha)

Secretos todavía **en claro** (la cripto va en Fase 2): primero ver el
aislamiento funcionando.

1. ✅ **`MountPlan` en `card-core::SomaSpec`.** Campo `mounts: MountPlan`
   declarativo: `home: HomeSpec` (`Heredar` | `Tmpfs{destino,size_bytes}` |
   `Subdir{origen,destino}`), `binds: Vec<BindSpec{origen, destino, ro}>`,
   `tmpfs: Vec<TmpfsSpec{destino, size_bytes}>`, `hide_home_real: Option<String>`
   (≈ `ProtectHome`). `#[serde(default)]` + sin `skip_serializing_if` (layout
   fijo de postcard en `WireCard`). `is_empty()` ⇒ no-op. 2 tests (default+compat,
   round-trip JSON+wire). _(Pendiente del enum: `HomeSpec::Overlay` — ya hay op
   `MountOverlay`, falta el ciclo upper/work; y `fuente_secreto` en `TmpfsSpec`
   llega con la cripto, Fase 2.)_
2. ✅ **El incarnator lo realiza.** `arje-incarnate` ganó ops
   `ChildPreExec::{MountTmpfs, BindMount}` (+ `mkdir_prefixes` async-signal-safe
   para crear mountpoints) y `ChildSetup::with_mount_plan(&MountPlan)` que compila
   el plan a ops en orden (home → tmpfs extra → binds), stateando el origen de
   cada bind en el padre. `incarnate_full` las inserta tras `make_root_private` y
   antes del drop de privilegios. Corre **sin root** mapeando uid→root-in-userns.
3. ✅ **Destino `Efimero` en `pacha-dotfiles`.** `MaterializeTarget::{Disco(p),
   Efimero(p)}` + `materializar_a`. El efímero escribe a una ruta tmpfs (RAM) que
   el manager bindea al `$HOME` del Card; no toca disco. _(La cripto en reposo
   sigue siendo Fase 2: hoy el tmpfs es la garantía de no-persistencia.)_
4. ✅ **Test madre headless (certificado por texto).**
   `mount_plan_aisla_secreto_entre_entes_y_no_toca_disco`: encarna dos Entes
   (`/bin/sh -c …`) en el mismo árbol real; el que tiene el secreto bindeado en su
   `$HOME` tmpfs lo lee (exit 0), el que no, no lo ve ni ve nada en su HOME
   (exit 0), y el `$HOME` de disco real queda vacío. Verificado verde en esta
   máquina (unprivileged userns activo); se auto-saltea donde el userns no está.

5. ✅ **Wiring `pacha`→Card (punto 3 original).** `AppSpec` ganó
   `fs_profile: Option<FsProfile>` (tipo plano en `pacha-core`, sin acoplar a
   card/dotfiles): `FsHome{Heredar|Tmpfs|Dotfiles}` + `secret_sets: Vec<String>`.
   Viaja solo en `Effect::SpawnApp`. `pacha-manager`/`LinuxSurfaces::incarnate`
   lo compila: `mount_plan_for` (pura) traduce el perfil a `card_core::MountPlan`
   (`Tmpfs`→`$HOME` privado vacío; `Dotfiles`→`$HOME` = staging), y
   `stage_secret_sets` materializa los sets (snapshot del `$HOME` actual) a un
   tmpfs en `XDG_RUNTIME_DIR` (RAM, no disco). Setea `namespaces.{mount,user}` +
   `soma.mounts` y encarna por el `Engine`. `respawn` (sin receta) cae a sin
   aislar. 3 tests (compilador por modo, staging-en-RAM, builder+serde en core).

**Fase 1 COMPLETA.** El aislamiento de FS por contexto va de la receta del
`Pacha` (`AppSpec.fs_profile`) hasta los `mount`/`tmpfs`/`bind` reales en el
namespace de la app, certificado por texto en cada capa. Lo único deliberadamente
fuera de Fase 1: la cripto en reposo (Fase 2) — hoy el tmpfs garantiza la
no-persistencia, pero los bytes en RAM están en claro.

## Fase 2 — Cifrado en reposo (secreto por defecto) — HECHA

1. ✅ **Capa de cifrado sobre `StoreObjetos`.** `Cifrador` sella cada objeto en
   un sobre AEAD `XChaCha20Poly1305` (`nonce(24) || ciphertext+tag`, nonce
   aleatorio por objeto — margen amplio bajo una clave de store de larga vida).
   El store gana `abrir_cifrado(raiz, Cifrador)` + `es_cifrado()`; `poner` sella,
   `traer` abre — `capturar`/`materializar` **no cambian de firma** (ven bytes).
   `None` = en claro (compat con stores existentes).
2. ✅ **Decisión #3 resuelta (opacidad de estructura).** El objeto **entero** se
   sella → contenido **y nombres** (los `Arbol`) van cifrados. La identidad
   (hash/ruta `aa/bbbb`) sigue siendo la del **claro** ⇒ grafo y dedup intactos,
   y un store en claro migra a cifrado sin recomputar hashes. Único leak: el hash
   del claro (la ruta) habilita *confirmación*, no lectura. La opacidad total
   (hash del sobre) rompería el dedup determinista → fuera de alcance.
3. ✅ **Clave desde la identidad (estilo `age`).** `Cifrador::derivar_de_seed(&[u8;32])`
   = HKDF-SHA256(seed, info=`"pacha-dotfiles-store-v1"`). La `seed` es la
   identidad Ed25519 del usuario (la que `agora-keystore` desbloquea). El
   `pacha-manager` gana `DotfilesCtx::new_cifrado(..., seed)`. **Nota de
   layering:** `pacha-dotfiles` (en `shared/`) NO depende de `agora-keystore`
   (en `03_ukupacha/`); recibe la seed ya desbloqueada de quien arma el contexto.
   *Desbloquear* la seed es Fase 3.
4. ✅ **Destino efímero descifra en RAM.** `materializar`/`materializar_a` →
   `traer` → abre el sobre en RAM; el `Efimero` escribe el claro al tmpfs. Nada
   en claro toca disco persistente.
5. ✅ **Tests (texto, sin render).** En `pacha-dotfiles`: round-trip cifrado +
   **grep que ni el contenido ni el nombre de archivo aparecen en claro** en los
   objetos de disco; clave equivocada ⇒ `DotError::Cripto` (AEAD falla);
   derivación determinista + separación de dominio (seed distinta no abre). En
   `pacha-manager`: `stage_into` sobre store cifrado descifra al staging RAM y
   deja el disco opaco.

## Fase 3 — Desbloqueo de la clave maestra (la decisión DIFÍCIL) — núcleo HECHO

"Un secreto para acceder a los secretos": la seed desbloqueada debe vivir en la
sesión para no re-preguntar en cada conmutación. **Decisión tomada (enchufable):
el kernel session keyring es el default**, con la *política* aislada tras un trait
para poder cambiarla sin tocar cripto ni manager.

1. ✅ **`pacha-llavero` (crate nuevo).** Trait [`Llavero`] (`guardar`/`recuperar`/
   `olvidar` un secreto de 32 bytes) = el **punto de conmutación de política**.
   Impls: `LlaveroKernel` (session keyring del kernel vía `add_key`/`keyctl`:
   vida atada a la sesión, se evapora al logout, no toca disco — análogo a
   `ssh-agent`) y `LlaveroMemoria` (RAM del proceso, tests/headless). `unsafe`
   contenido sólo acá (las otras crates pacha siguen `forbid(unsafe_code)`).
   **Desacople de la cripto a propósito:** maneja 32 bytes opacos, no conoce
   `Cifrador`/HKDF — cambiar de backend (PAM/TPM/passphrase+Argon2/greeter) NO
   toca a quien lo usa. 2 tests (memoria + round-trip REAL por el session keyring).
2. ✅ **Bridge en `pacha-manager`.** `DotfilesCtx::desde_llavero(..., llavero,
   nombre)` abre el store cifrado tomando la seed del llavero; `Ok(None)` si no
   está cacheada (→ hay que desbloquear primero). `DotfilesCtx::cachear_seed`
   guarda la seed (ya obtenida de agora) para la sesión. Test: `None` sin caché;
   tras `cachear_seed` la MISMA seed abre el MISMO store; seed equivocada no abre.

**Lo que queda (glue dependiente del entorno, NO mecanismo):** el orquestador (un
binario/daemon, o `mirada-greeter` al login) debe, la primera vez: pedir la
passphrase, abrir `agora-keystore` (que ya cifra la seed en disco con
Argon2+ChaCha) y llamar `cachear_seed`. Eso es interactivo/contextual y no se
certifica headless; el mecanismo entero por debajo ya está y testeado. La elección
de endurecimiento (TTL de la key, `KEYCTL_SET_TIMEOUT`, TPM-seal) queda abierta
pero NO bloquea Fase 4: la seed ya se desbloquea y retiene de forma segura.

## Fase 4 — Compartir/publicar + transporte remoto — HECHA

1. ✅ **Remoto / push por set-difference.** `alcanzables(store, desde)` recorre el
   cono siguiendo las aristas `hijos` (uniforme: commit→`[raiz,padre]`,
   árbol→entradas, índice→trozos; **no parsea `Arbol`**, igual que el MARK del GC
   del kernel). `empujar(origen, destino, desde) -> PushStats{copiados,
   ya_presentes}` copia sólo lo que al destino le falta (como `git push`; espeja
   `akasha`). Reusa `traer`/`poner`: descifra en RAM y re-sella con la clave del
   **destino** ⇒ el remoto puede tener **otra clave** (o ninguna) y queda opaco;
   la identidad (hash del claro) cruza stores, así el dedup también. Tests: delta
   real (cambia 1 archivo → copia sólo su camino, el resto `ya_presentes`),
   idempotencia, cruce de claves con remoto opaco.
2. ✅ **Publicar / re-cifrar a destinatarios (estilo `age`).** `publicar_para(
   store, raiz, &[pub_x25519]) -> SobreCompartido`: bundlea el cono en claro,
   lo sella con una clave de contenido fresca, y **envuelve** esa clave a cada
   destinatario (efímero por destinatario + ECDH X25519 + HKDF + AEAD = una
   stanza age). `abrir_compartido(sobre, &mi_seed)` prueba las stanzas, verifica
   integridad por hash y devuelve los objetos; `importar(store, objetos)` los
   re-`poner`á (re-cifrando con la clave del receptor). `clave_publica_de_seed`
   da tu pública (X25519 derivada de la misma seed Ed25519, patrón de
   `ayni-crypto`). Verbo explícito: re-cifrar a otros, jamás el default. Tests:
   sólo el destinatario abre (un no-destinatario falla), multi-destinatario,
   sobre serializado sin claro filtrado. Exportar **en claro** = `materializar`
   a un dir (ya existe; no se duplica).
3. ✅ **Bridge en `pacha-manager`.** `DotfilesCtx::{publicar_set, empujar_set}`
   capturan el set actual y delegan. Test end-to-end por el manager.

**Layering:** todo vive en `pacha-dotfiles` (en `shared/`), tomando **claves
X25519 crudas** — sin depender de `agora`. Convertir la identidad Ed25519 de un
contacto a su X25519 pública (y obtener la libreta de contactos) es del
orquestador. Pendiente menor: el transporte de red real (hoy "remoto" = otro
`StoreObjetos` en otra ruta/disco, que es exactamente lo que el PLAN pedía para
empezar).

## Fase 5 — Refinamientos

- **Splicing por ruta:** honrar `ModoGestion` por-ruta (recapturar sólo subárboles
  Rastreados, conservar los Fijado pinneados). Hoy se recaptura el set entero.
- **Persistencia daemon + CLI:** catálogo de sets + cabezas en
  `pacha-manager/server.rs`; comandos `pacha dotfiles add/snapshot/restore/publish`.
- **UI:** proyectar los sets como Mónadas en nahual (`NouserSource`) para browsear
  — capa de vista, no de verdad.

## Decisiones abiertas

1. ~~**Desbloqueo de la clave maestra** (Fase 3) — la grande.~~ **RESUELTA:**
   session keyring del kernel como default, **enchufable** tras el trait `Llavero`
   (`pacha-llavero`). Queda abierto sólo el endurecimiento (TTL/TPM) y el front de
   desbloqueo inicial (greeter/passphrase), no el mecanismo.
2. **Granularidad del aislamiento:** un mount ns por pacha vs uno por Card.
   ¿Secretos scoped a pacha, a un set de apps, o a Cards individuales?
3. **Opacidad de estructura:** ¿el árbol/nombres de archivo van cifrados (oculta
   qué dotfiles tenés) o sólo los blobs (filtra estructura pero permite dedup
   entre usuarios)? Afecta si el hash-identidad es del claro o del sobre.
4. **uid por pacha:** ¿usar `run_as` para correr cada contexto como un uid
   distinto (separación dura) o un solo uid con mount ns por app?
