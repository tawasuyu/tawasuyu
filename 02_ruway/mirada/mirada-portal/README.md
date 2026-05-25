# mirada-portal

Backend de `xdg-desktop-portal` para el escritorio **carmen**. Implementa
`org.freedesktop.impl.portal.Settings` y publica un único namespace:
`org.freedesktop.appearance`.

## Qué resuelve

GTK y Qt leen su configuración de sitios incompatibles entre sí. Pero
**ambos** —además de Firefox y Chromium— consultan el portal de
FreeDesktop para saber:

- `color-scheme` — claro (`2`) u oscuro (`1`),
- `accent-color` — el color de acento como `(ddd)` RGB,
- `contrast` — contraste alto (`1`) o normal (`0`).

`mirada-portal` responde esas tres claves a partir del tema activo de
`nahual` y, cuando el tema cambia, emite `SettingChanged`: todo el
ecosistema voltea en vivo, **sin tocar un solo archivo de config de las
apps**.

## Fuente del tema

El daemon lee `$XDG_CONFIG_HOME/nahual/theme` (el archivo que persiste
`nahual-theme`, con el nombre del preset activo) y lo vigila con
`notify`. La traducción nombre → hechos del portal está en
[`src/theme_facts.rs`], que espeja `nahual_theme::Theme::all()` sin
enlazar GPUI.

## Arquitectura

Esto es el **backend** del portal. El frontend genérico
`xdg-desktop-portal` (paquete agnóstico, liviano) enruta las llamadas de
las apps hacia este backend según el archivo `mirada.portal`. No hay que
implementar el frontend.

## Instalación de los archivos de `data/`

```sh
install -Dm644 data/mirada.portal \
    /usr/share/xdg-desktop-portal/portals/mirada.portal
install -Dm644 data/mirada-portals.conf \
    /usr/share/xdg-desktop-portal/mirada-portals.conf
install -Dm644 data/org.freedesktop.impl.portal.desktop.mirada.service \
    /usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.mirada.service
install -Dm755 target/release/mirada-portal /usr/bin/mirada-portal
```

El frontend casa `UseIn=mirada` contra `XDG_CURRENT_DESKTOP`, así que
carmen debe exportar `XDG_CURRENT_DESKTOP=mirada`. Alternativamente, el
`mirada-portals.conf` lo fuerza con `default=mirada`.

`mirada-portal` se puede arrancar desde `~/.config/mirada/autostart` o
dejar que el frontend lo active por D-Bus (de ahí el `.service`).

## Smoke test (sin frontend ni apps GTK)

Con un bus de sesión vivo, el backend se puede interrogar directo:

```sh
busctl --user introspect org.freedesktop.impl.portal.desktop.mirada \
    /org/freedesktop/portal/desktop
busctl --user call org.freedesktop.impl.portal.desktop.mirada \
    /org/freedesktop/portal/desktop \
    org.freedesktop.impl.portal.Settings ReadAll as 0
```

Cambiar `~/.config/nahual/theme` debe disparar una señal `SettingChanged`
(observable con `busctl --user monitor`).

## Límite conocido (v1)

El portal `org.freedesktop.appearance` sólo lleva claro/oscuro + acento +
contraste. **No** lleva la paleta completa de `nahual`. Para recolorear
GTK/Qt a los colores exactos del tema hace falta, además, inyección de
entorno + CSS generado en el `spawn` de carmen — siguiente paso del plan.
