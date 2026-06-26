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

## v0 eager → activación perezosa (futuro)

Hoy los shims se declaran como entes del `genesis` y se **encarnan al
boot** (eager). El upgrade natural es **activación perezosa** al estilo
D-Bus: registrar los nombres `org.freedesktop.*` como activables y
spawnear el shim al primer request. Cuando exista esa capa, el fragmento
declara *disponibilidad* en vez de *spawn*, y arrancar la sesión GNOME no
cuesta 12 procesos que quizá nadie consulte.

## Cómo añadir una sesión

1. Crear `session-<nombre>.card.json`: una `EntityCard` `Virtual` con los
   entes nuevos en `genesis` (ids ULID únicos).
2. Añadir un test en `../../init/arje-zero/tests/seeds.rs` que valide el
   fragmento y compruebe sus entes clave.
3. Documentarlo en la tabla de arriba.
