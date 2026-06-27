# Cola de verificación — semana 20→26 jun (486 commits, ninguno aprobado en metal aún)

Checklist marcable derivado del inventario (reporte sesión `86cd6c79`). Clasificado
en **falta probar** (necesita tu ojo/metal/hardware) y **falta enchufar** (wiring que
puedo codear). Marcá `[x]` lo aprobado en metal. Orden de "aprobación" al final.

Regla 8 de `CLAUDE.md`: certifico con tests/stats donde se pueda; el render se mira
sólo cuando es un visual nuevo no certificable de otra forma.

---

## 1. arje — init/PID 1, arranque ✅ (atendido 2026-06-27)
- [x] Seed de producción `arje-tawasuyu.card.json` (génesis splash→mirada-greeter), 7/7 tests
- [x] Cadena splash→handoff certificada en QEMU (`test-arje-splash-qemu.sh`)
- [ ] **Falta en metal:** arranque real (no QEMU); watchdog PID1; activación perezosa de shims arje-compat
- [ ] **Enchufar (histórico):** tabla de capacidades por bytecode hash

## 2. mirada — compositor/escritorio (122 commits, el grueso)
> `scripts/actualizar-mirada.sh` rebuildea, luego login. Diag: `scripts/diag-mirada.sh`.
- [ ] Glassmorphism (menú/barra frosted) — confirmar blur en metal
- [ ] Cubo Win+Tab — verificado headless, falta verlo en metal
- [ ] Prezi / vista espacial — rotación viva con handoff a metal; mapa Prezi editable en wawa-panel
- [ ] FUS sesiones: login→lock→switch-user→logout completo en metal
- [ ] Efectos nuevos: corner_radius GPU vía GlesRenderer
- [x] **Wallpaper video por salida (worker por monitor):** ya implementado — cada `OutputCtx` corre su propio `VideoWallpaper` (drm_backend). Falta confirmar multi-monitor en metal.
- [x] **ENCHUFADO 2026-06-27 — wallpaper ESTÁTICO por salida:** `mirada-wallpaper` ya no rechaza `output != ""`; con un conector (`output: "DP-1"`) reescribe el `OutputOverride` de esa salida en `config.ron` (editor RON quirúrgico `set_output_wallpaper_path`, preserva comentarios). 23/23 tests verdes.
- [x] **Plugins WASM grants firmados:** ya implementado y testeado (27/27, `mirada-plugin-host` trust.rs — Ed25519 sobre blake3(wasm)‖caps, fail-closed). Falta probar hot-reload con catálogo real.
- [ ] Sesiones remotas waypipe: contra host remoto real
- [ ] Sistema: night-light/DPMS/idle/auto-lock en metal

## 3. pata — barra/panel/host de shell (42 commits)
> Test aislado: `scripts/test-pata-mirada.sh`.
- [ ] FUS 16ª applets (volumen-por-app, Wi-Fi, BT, MPRIS, polkit, OSD, notif+DND, calendario, energía, batería) con hardware real
- [ ] Notificaciones: triage semántico con LLM real; elegir fuente RAG (willay vs paloma)
- [ ] Dientes/dock-rail: reordenar y ver re-publicación
- [ ] Switcher de escritorios: latencia en metal

## 4. llimphi — motor gráfico (37 commits)
> `cargo run -p llimphi-anim-studio --release` · `-p llimphi-voxel-studio --release`
- [ ] Máquina de animación Rive (Tiers 1→5) — usarla en una app real (nada la consume fuera del studio)
- [ ] anim-studio F1/F2/F3 — exportar/consumir desde otra app
- [ ] llimphi-lottie — con archivos Lottie reales
- [ ] voxel-studio — autoría + render de showreel (editor independiente, no enchufado a producto)

## 5. cosmos — esfera celeste 3D + rueda (22 commits)
> `cargo run -p cosmos-app-llimphi --release`
- [ ] Esfera 3D: legibilidad/rendimiento en metal
- [ ] Rueda rediseñada: validación visual con carta real

## 6. supay — doom/raycaster (37 commits)
> `cargo run -p supay-doom --release` (F3 = wgpu 2.5D)
- [ ] Jugar partida completa con WAD real
- [ ] BSP: comparar vs ground-truth Freedoom

## 7. paloma — correo soberano (12+ commits)
> `cargo run -p paloma-app`
- [ ] Correo LLM-nativo: cuenta real + credenciales
- [ ] Rail P2P (Ed25519/agora/DHT/Suyu/web-of-trust): **dos nodos reales hablándose**

## 8. willay — centro de eventos (12 commits)
> `cargo run -p willay-panel`
- [ ] Feed en vivo con eventos reales fluyendo
- [x] **willay-daemon autostart:** registrado en churay (`churay-core/base.rs:141` → línea en `~/.config/mirada/autostart`); arranque certificado por test e2e `cargo test -p willay-daemon --test socket` (1/1 verde: emite y consulta por socket).

## 3.b — pata: triage notif con fuente RAG (evaluado 2026-06-27)
> NO enchufado. El selector willay/paloma existe en `pata-llimphi` (sidebar de correo),
> no en `pata-notify-triage`. Meterlo ahí es **dudoso**: `RagMotor::ask` devuelve una
> respuesta redactada con su propio LLM (pesado, callback→async), y un título de 8
> palabras no lo amerita. El gap real del triage es "falta probar con LLM/embeddings
> REALES" (credenciales/daemon verbo = metal), no el wiring. **Recomendación: diferir.**

## 9. pluma + takiy — versionado de proyectos
- [ ] pluma: ciclo crear→ramificar→merge→push; verificar no-regresión tras quitar sled (.pluma única persistencia)
- [ ] takiy: grabar→editar→versionar

## 10. churay — instalador/actualizador (12 commits)
> `cargo run -p churay --release`
- [ ] Camino Windows (spike, sin validar)

## 11. shuma — gateway móvil + flota (≈20 commits)
> `cargo run -p shuma-gateway` → navegador en `/term`
- [ ] matilda: contra una flota SSH real

## 12. Pase masivo "moderniza UI" (~35 apps, 06-26)
- [ ] Abrir cada app y mirar que empty-state/pop-in/toasts/skeleton no rompan layout

---

## Orden recomendado de aprobación
1. ~~arje + mirada handoff~~ (arje hecho) — la capa que tira la sesión ("se sigue cayendo")
2. mirada FUS sesiones (login/lock/switch/logout) — flujo crítico diario
3. pata applets con hardware real
4. Resto (cosmos, pluma, paloma P2P, voxel-studio, churay-Windows) — funcionales aislados, no bloquean el escritorio
