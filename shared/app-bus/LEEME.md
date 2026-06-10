# app-bus — bus de eventos in-proc para apps Llimphi

Bus de **publish/subscribe** tipado, síncrono y en memoria, para coordinar apps
dentro del mismo proceso. Cubre tres familias de eventos transversales: **foco**
(qué app/ventana lo tiene), **navegación** (abrir/cerrar/enfocar una app o ruta)
y **notificaciones** efímeras.

## Qué expone

- `AppBus` — handle compartible (`Clone`, internamente `Arc<RwLock<…>>`).
- `Event` — enum transversal: `FocusChanged` / `Navigate` / `CloseApp` / `Notify`.
- `NotifyLevel` — `Info` / `Warn` / `Error`.
- `Subscription` — guard RAII: al soltarlo se cancela la suscripción.
- `publish(Event)` entrega de forma síncrona a todos los suscriptores; entrega
  anidada si un callback publica desde su propio handler.

## No-objetivos

- No es un bus interproceso ni de red (eso es Akasha / app-channel).
- No persiste eventos ni garantiza entrega tras reinicio.
- No ordena por prioridad.

## Estado (2026-05-31)

### Hecho
- Bus pub/sub completo: `publish`, `subscribe`, cancelación por guard RAII.
- Enum `Event` con foco/navegación/cierre/notificaciones.
- Consumido por `launcher-llimphi` (dispara navegación/lanzamiento).

### Pendiente
- Adaptador multiproceso (hoy estrictamente in-proc).
- Entrega diferida / cola (hoy reentrancia anidada síncrona).
- Filtrado por tipo de evento en `subscribe` (hoy el callback filtra).

## Lugar en el repo

`shared/app-bus` — canal in-proc de grano fino. El plano de control a más alto
nivel es `shared/sandokan` (ver su SDD.md).
