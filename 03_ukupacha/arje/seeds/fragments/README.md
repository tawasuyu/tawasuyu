# `seeds/fragments/` — fragmentos de sesión

Un **perfil de arranque** = la Tarjeta Semilla base (`arje-host` /
`arje-qemu`, en `../`) **+** opcionalmente los entes de una *sesión*.

La base ya es el perfil **mirada**: trae los inits básicos (agetty,
network-up, splash) y el **greeter** (`mirada-greeter-llimphi`), que es
el DM. Mirada es nativo: no necesita servicios de sistema extra.

Un fragmento de sesión es una `EntityCard` `Virtual` cuyo `genesis`
contiene **sólo los entes que esa sesión añade** sobre la base. El
compositor [`profile::overlay_session`](../../init/arje-zero/src/profile.rs)
los anexa al `genesis` de la base (dedup por `label`, idempotente).

## Catálogo

| Fichero | Sesión | Qué añade |
|---|---|---|
| `session-gnome.card.json` | `gnome` | Los shims D-Bus de `arje-compat` (logind, hostnamed, timedated, localed, polkit, systemd1, journald, resolved, machined, policy-provider, notify, timer) para que una sesión GNOME lanzada desde el greeter encuentre los `org.freedesktop.*` que consulta al arrancar. |

> `mirada` **no** tiene fragmento: es la base sola. No hay overlay.

## Selección

El punto de selección es el cmdline del kernel:

```
arje.session=gnome      # base + session-gnome
arje.session=mirada     # base sola (= no pasar nada)
```

En desarrollo, la env `ARJE_SESSION` tiene prioridad sobre el cmdline:

```sh
ARJE_SESSION=gnome arje-zero --dev
```

`arje-zero` busca el fragmento en `<dir-de-la-seed>/fragments/`
(`/ente/fragments/` en prod, `seeds/fragments/` o `fragments/` en dev).
Si la sesión solicitada no tiene fragmento, o el fichero no valida, se
arranca la **base** — un perfil mal nombrado nunca deja sin arranque.

La elección *fina* de qué sesión iniciar tras autenticar es del greeter
(que ya es el DM). Este overlay sólo decide qué backends de sistema
están presentes para esa sesión.

## Dos vías de activación

El mismo fichero de fragmento sirve a dos momentos:

1. **Boot-time (overlay).** `arje.session=gnome` en el cmdline → la
   Semilla nace con los shims ya en su `genesis`
   (`profile::overlay_session`). Para esto el fragmento vive en
   `seeds/fragments/` y se compone al construir la seed.

2. **Login-time (bundle).** El greeter (el DM), al elegir una sesión que
   necesita backends de sistema, manda **un** request al bus de arje:

   ```rust
   // mirada-greeter / mirada-compositor, tras elegir la sesión gnome:
   arje_bus::BusRequest::SpawnCardFromDisk { name: "session-gnome".into() }
   ```

   `arje-zero` reconoce el fragmento `Virtual` con `genesis` como un
   **bundle** y encarna sus miembros (los shims), no el envoltorio
   (`graph::bus_mediator::expand_disk_bundle`). Para esta vía el
   fragmento y los binarios de los shims se instalan con:

   ```sh
   scripts/install-arje-session-gnome.sh --system
   # → /usr/local/lib/arje/arje-*-compat  +  /etc/arje/cards.d/session-gnome.json
   ```

   Esta es la vía que cierra el acople boot↔login: los backends de GNOME
   se levantan **cuando el usuario elige esa sesión**, no eagermente al
   arranque. El hook vive en `mirada-greeter`
   (`src/arje_session.rs`): al elegir una sesión con `profile_for(...) =
   Some("gnome")`, manda ese request antes de emitir el `SessionTicket`
   (best-effort, con tope de espera).

## v0 eager → activación perezosa (futuro)

Hoy los shims se declaran como entes del `genesis` y se **encarnan al
boot** (eager). El upgrade natural es **activación perezosa** al estilo
D-Bus: registrar los nombres `org.freedesktop.*` como activables y
spawnear el shim al primer request. Cuando exista esa capa, el fragmento
declara *disponibilidad* en vez de *spawn*, y arrancar la sesión GNOME no
cuesta 12 procesos que quizá nadie consulte.

## Instalación

`scripts/install-arje-session-gnome.sh`:

- `--system` instala los shims (`<prefix>/lib/arje/arje-*-compat`, 0755) y
  el bundle (`/etc/arje/cards.d/session-gnome.json`) en el sistema actual
  — la vía login-time del greeter.
- `--emit-flags` compila los shims estáticos (musl) y emite los flags
  `--asset`/`--bin` para sumar a un `arje-installer` de host (arranque
  nativo). El installer recoge los execs de las cards de
  `/etc/arje/cards.d/` y hornea sus binarios (`lib.rs::collect_card_execs`).

## Pendiente

- **Mapeo sesión→perfil** en `mirada-greeter::arje_session::profile_for`
  hoy es heurístico (detecta GNOME por `exec`/`name`). Cuando aparezca
  otra sesión con backends arje, conviene una tabla o un campo del
  `.desktop` en vez de la heurística.

## Cómo añadir una sesión

1. Crear `session-<nombre>.card.json`: una `EntityCard` `Virtual` con los
   entes nuevos en `genesis` (ids ULID únicos).
2. Añadir un test en `../../init/arje-zero/tests/seeds.rs` que valide el
   fragmento y compruebe sus entes clave.
3. Documentarlo en la tabla de arriba.
