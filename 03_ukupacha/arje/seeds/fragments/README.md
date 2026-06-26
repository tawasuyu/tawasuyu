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
   (`src/arje_session.rs::reconcile`): en cada login levanta el bundle de
   la sesión elegida y **baja** los otros perfiles opcionales que hayan
   quedado vivos. Best-effort, con tope de espera.

3. **Teardown (login-time).** Inverso de (2): al volver de gnome a mirada,
   el greeter manda

   ```rust
   arje_bus::BusRequest::StopCardFromDisk { name: "session-gnome".into() }
   ```

   `arje-zero` marca los miembros vivos del bundle y les manda SIGTERM **sin
   reiniciarlos** (su supervisor `Restart` no los revive — `graph::stopping`
   + `on_death`). Idempotente: miembros ya muertos se ignoran.

## Activación perezosa (lazy) — `arje-activate`

Las vías 1–3 son **eager**: al elegir gnome arrancan los 12 shims juntos,
aunque la app sólo consulte 2 o 3. La vía **lazy** los arranca on-demand,
sin que arje deje de ser la única autoridad de spawn/supervisión.

Cómo encaja (es una *excepción calculada*, no un choque con el modelo):
arje **no** corre un dbus-daemon — los shims hablan al *system bus del
host*. Así que la activación la dispara el **dbus-daemon del host**:

1. Una app pide `org.freedesktop.login1`. El host lee el `.service` de
   activación → `Exec=/usr/lib/arje/arje-activate compat-logind`.
2. `arje-activate` (un cliente mínimo del bus de arje) manda
   `SpawnCardFromDisk { name: "compat-logind" }` y **sale** — no reclama el
   nombre.
3. `arje-zero` encarna el shim (con su `Restart`/telemetría, en el grafo);
   el shim, al vivir, reclama el nombre y el host entrega el mensaje.

El dbus-daemon queda como **sensor de borde** (traduce "pidieron X" en un
evento del bus de arje); arje sigue encarnando y supervisando. Es el patrón
`SystemdService=` de D-Bus, con arje en lugar de systemd. El único estado
cedido al puente es *qué nombres existen y quién los reclama* (vive en el
dbus-daemon, no en la frontera de capacidades de arje) — concesión acotada
al puente compat, que ya es la zona de excepción para protocolos ajenos.

Instalación (`--lazy`): instala `arje-activate`, una card por shim en el
store, un `.service` de activación por nombre, y el marcador
`/etc/arje/session-gnome.lazy`. El greeter ve el marcador y **no** levanta
gnome eager (`arje_session::is_lazy`), pero **sí** lo baja al salir
(teardown por label, vía 3). Contrato de deploy: `arje-zero` debe correr con
`ENTE_BUS_SOCK=/run/arje/bus.sock` (el path que `arje-activate` usa cuando
el dbus-daemon del host lo invoca sin heredar el env del fractal).

```sh
scripts/install-arje-session-gnome.sh --lazy   # requiere jq + dbus-daemon de sistema
```

## Instalación

`scripts/install-arje-session-gnome.sh`:

- `--system` instala los shims (`<prefix>/lib/arje/arje-*-compat`, 0755) y
  el bundle (`/etc/arje/cards.d/session-gnome.json`) en el sistema actual
  — la vía login-time del greeter.
- `--emit-flags` compila los shims estáticos (musl) y emite los flags
  `--asset`/`--bin` para sumar a un `arje-installer` de host (arranque
  nativo). El installer recoge los execs de las cards de
  `/etc/arje/cards.d/` y hornea sus binarios (`lib.rs::collect_card_execs`).

## Cómo añadir una sesión

1. Crear `session-<nombre>.card.json`: una `EntityCard` `Virtual` con los
   entes nuevos en `genesis` (ids ULID únicos).
2. Añadir un test en `../../init/arje-zero/tests/seeds.rs` que valide el
   fragmento y compruebe sus entes clave.
3. Documentarlo en la tabla de arriba.
