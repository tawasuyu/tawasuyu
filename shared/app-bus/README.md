# app-bus — in-proc event bus for Llimphi apps

A typed, synchronous, in-memory **publish/subscribe** bus to coordinate apps
within the same process. It covers three families of cross-cutting events: **focus**
(which app/window holds it), **navigation** (open/close/focus an app or route)
and ephemeral **notifications**.

## What it exposes

- `AppBus` — shareable handle (`Clone`, internally `Arc<RwLock<…>>`).
- `Event` — cross-cutting enum: `FocusChanged` / `Navigate` / `CloseApp` / `Notify`.
- `NotifyLevel` — `Info` / `Warn` / `Error`.
- `Subscription` — RAII guard: dropping it cancels the subscription.
- `publish(Event)` delivers synchronously to all subscribers; nested delivery
  if a callback publishes from its own handler.

## Non-goals

- It is not an inter-process or network bus (that is Akasha / app-channel).
- It does not persist events nor guarantee delivery after restart.
- It does not order by priority.

## Status (2026-05-31)

### Done
- Complete pub/sub bus: `publish`, `subscribe`, cancellation via RAII guard.
- `Event` enum with focus/navigation/close/notifications.
- Consumed by `launcher-llimphi` (fires navigation/launch).

### Pending
- Multi-process adapter (today strictly in-proc).
- Deferred delivery / queue (today synchronous nested reentrancy).
- Filtering by event type in `subscribe` (today the callback filters).

## Place in the repo

`shared/app-bus` — fine-grained in-proc channel. The higher-level control plane
is `shared/sandokan` (see its SDD.md).
