# mirada-portal

> `xdg-desktop-portal` backend for [mirada](../README.md).

Implements the freedesktop portal protocol on top of [`mirada-compositor`](../mirada-compositor/README.md): file pickers, screenshare, open-uri, screenshot. Any app using portal APIs (Firefox, Chromium, GTK apps) works on the carmen desktop without modification.

## Deps

- [`mirada-protocol`](../mirada-protocol/README.md), [`mirada-link`](../mirada-link/README.md)
- `zbus` (D-Bus)
