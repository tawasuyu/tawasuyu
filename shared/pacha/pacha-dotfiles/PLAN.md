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

## Fase 1 — Aislamiento de FS + destino efímero (siguiente, MAYOR señal)

Secretos todavía **en claro** (la cripto va en Fase 2): primero ver el
aislamiento funcionando.

1. **`MountPlan` en `card-core::SomaSpec`.** Nuevo campo `mounts: MountPlan`
   declarativo: `home: HomeSpec` (heredar | tmpfs | subdir-del-pacha | overlay),
   `binds: Vec<BindSpec{origen, destino, ro}>`, `tmpfs: Vec<TmpfsSpec{destino,
   size, fuente_secreto: Option<…>}>`, `oculta_home_real: bool` (≈ `ProtectHome`).
   El Admin (incarnator de sandokan/arje) lo compila a `unshare(CLONE_NEWNS)` +
   `mount`/`pivot_root`/`bind`. `namespaces.mount` ya existe como toggle.
2. **Destino `Efimero` en `materializar`.** `pacha-dotfiles` gana un target:
   `MaterializeTarget::{Disco(home), Efimero(mount_fd|ruta_tmpfs)}`. El efímero
   escribe el árbol descifrado a un tmpfs montado en el namespace del Card, no al
   `$HOME` real.
3. **Wiring pacha→Card.** La receta de apps de un `Pacha` (`AppSpec`) gana
   referencia opcional a un perfil de FS + qué secret-sets montar. `LinuxSurfaces`
   arma el `SomaSpec.mounts` al `incarnate`.
4. **Test headless:** encarnar dos Cards en un mismo pacha; uno con el secret-set
   montado y un HOME tmpfs propio, otro sin nada. Asertar (por inspección de
   `/proc/<pid>/mountinfo` o ejecutando un binario que lista su HOME) que el
   primero ve el archivo y el segundo NO, y que nada apareció en el disco real.

## Fase 2 — Cifrado en reposo (secreto por defecto)

1. **Capa de cifrado sobre `StoreObjetos`.** Cada blob se guarda en un sobre
   cifrado (AEAD; nonce + ciphertext). El árbol/commit pueden ir cifrados también
   o sólo los blobs hoja (decidir: ¿metadatos de estructura visibles? — ver
   decisiones abiertas). La identidad del objeto sigue siendo el hash del
   contenido **en claro** (para dedup) o del sobre (para opacidad) — decisión.
2. **Clave desde agora.** Derivar una clave simétrica/X25519 de la identidad
   Ed25519 del usuario (`format/firma.rs`, `agora-keystore`). Estilo `age`.
3. **`capturar`/`materializar` no cambian de firma** — ven bytes; el sobre lo
   pone/quita el store. El destino efímero descifra en RAM.
4. **Test:** round-trip cifrado (capturar→store opaco en disco, grep que el
   contenido en claro NO aparece en los bytes del objeto; materializar reproduce
   el original).

## Fase 3 — Desbloqueo de la clave maestra (la decisión DIFÍCIL)

Para descifrar al vuelo hace falta la clave desbloqueada en la sesión: "un secreto
para acceder a los secretos". Opciones a evaluar: keyring del kernel
(`add_key`/`keyctl`), PAM al login, passphrase + Argon2, TPM/sello,
`agora-keystore` desbloqueado por mirada-greeter. Define qué tan fuerte es el
modelo entero. **No avanzar Fases 4+ sin cerrar esto.**

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
