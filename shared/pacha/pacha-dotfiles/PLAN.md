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

## Fase 3 — Desbloqueo de la clave maestra (la decisión DIFÍCIL)

Para descifrar al vuelo hace falta la clave desbloqueada en la sesión: "un secreto
para acceder a los secretos". Opciones a evaluar: keyring del kernel
(`add_key`/`keyctl`), PAM al login, passphrase + Argon2, TPM/sello,
`agora-keystore` desbloqueado por mirada-greeter. Define qué tan fuerte es el
modelo entero. **No avanzar Fases 4+ sin cerrar esto.**

Estado: la cripto (Fase 2) ya consume una `seed` desbloqueada vía
`derivar_de_seed`/`new_cifrado`; lo que falta es **de dónde sale desbloqueada** —
hoy el caller la provee. `agora-keystore` ya cifra la seed en disco con passphrase
(Argon2+ChaCha); el eslabón pendiente es **quién** pide la passphrase y **dónde**
queda la seed viva (keyring del kernel vs. proceso del manager).

## Fase 4 — Compartir/publicar + transporte remoto

- **Publicar:** re-cifrar un set (o instantánea) a un conjunto de destinatarios
  (claves públicas agora), o exportar en claro — verbo explícito en `pacha-cli`.
- **Remoto:** "push" = set-difference de hashes (lo que el remoto no tiene), como
  `git push`; espeja lo que `akasha` hace en wawa para el grafo. Empezar con un
  store en otra ruta/disco.

## Fase 5 — Refinamientos

- **Splicing por ruta:** honrar `ModoGestion` por-ruta (recapturar sólo subárboles
  Rastreados, conservar los Fijado pinneados). Hoy se recaptura el set entero.
- **Persistencia daemon + CLI:** catálogo de sets + cabezas en
  `pacha-manager/server.rs`; comandos `pacha dotfiles add/snapshot/restore/publish`.
- **UI:** proyectar los sets como Mónadas en nahual (`NouserSource`) para browsear
  — capa de vista, no de verdad.

## Decisiones abiertas

1. **Desbloqueo de la clave maestra** (Fase 3) — la grande.
2. **Granularidad del aislamiento:** un mount ns por pacha vs uno por Card.
   ¿Secretos scoped a pacha, a un set de apps, o a Cards individuales?
3. **Opacidad de estructura:** ¿el árbol/nombres de archivo van cifrados (oculta
   qué dotfiles tenés) o sólo los blobs (filtra estructura pero permite dedup
   entre usuarios)? Afecta si el hash-identidad es del claro o del sobre.
4. **uid por pacha:** ¿usar `run_as` para correr cada contexto como un uid
   distinto (separación dura) o un solo uid con mount ns por app?
