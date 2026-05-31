# Inventario de apps gioser — % de avance y qué falta

> Estado al **2026-05-31**. Recalculado contra **núcleo funcional usable hoy**, no
> contra la visión total (ese era el error del inventario anterior: una app que ya
> hace su trabajo salía ~30% porque le faltaba el 70% de *sueños futuros*).

## Cómo se lee el %

`% core` mide si la app **hace su trabajo central y se usa hoy**. Rúbrica:

- Lógica de dominio testeada — **40**
- App/UI end-to-end del caso principal — **30**
- Persistencia / I/O reales (no mocks) — **15**
- Robustez y pulido (errores, edge cases, perf) — **15**

Lo pendiente se parte en dos cubetas:

- **Falta core** → *descuenta* del %. Sin esto el caso principal cojea.
- **Visión / stretch** → *no descuenta*. Integraciones futuras, otras plataformas,
  features extra, roadmaps ambiciosos.

Los % son juicio anclado en código + git + tests, redondeados a múltiplos de 5. No es auditoría formal.

## 00_unanchay — PERCIBIR

| App | % core | Falta para cerrar el core | Visión / stretch (no descuenta) |
|---|---|---|---|
| **pineal** | 92% | Una viz densa real que ejercite el camino GPU end-to-end | Consumidores vivos (cosmos/nakui/takiy), vista 3D |
| **khipu** | 88% | Sync bidireccional + resolución de conflictos | Endurecer la malla DHT en WAN, escala |
| **pluma** | 85% | Cerrar kernels notebook python/wasm; foreign-docx completo | Split tullpu+Cámara del deck, federación minga |
| **rimay** | 85% | Gating de permiso de descarga del modelo | Compilar a WASM (Wawa), vertiente lingüística quechua |
| **chaka** | 80% | REPLACE, ficheros indexed/relative | Dialectos no-COBOL, target WASM (no_std), CICS/SQL/Db2 |
| **puriy** | 78% | Cerrar las APIs Web restantes + conformance | JIT, HTTP2/3, identidad agora, bare-metal Wawa |

## 01_yachay — CONOCER

| App | % core | Falta para cerrar el core | Visión / stretch |
|---|---|---|---|
| **tinkuy** | 88% | Escenas editables desde DSL/grafo | Subir el techo de partículas del visor, más fuerzas nativas |
| **dominium** | 85% | Exponer SimParams/ZWeights restantes como dato | Sprites reales, escenarios sociales reproducibles, demo web |
| **cosmos** | 82% | Edición rica de cartas in-situ (hoy parte vía JSON manual) | Viz astrológicas avanzadas, corpus humano de interpretación |
| **nakui** | 70% | Editor de fórmulas en UI + persistencia WAL desde UI + vista formulario | Verticales ERP reales, motor `yupay` (es/qu) |
| **iniy** | 65% ⚠️ | Pipeline e2e *probado* + NLI local sólido (hoy piezas sueltas/mock) | Opiniones multinomiales, más operadores SL, piloto real |

## 02_ruway — HACER

| App | % core | Falta para cerrar el core | Visión / stretch |
|---|---|---|---|
| **ayni** | 88% | NAT traversal (deuda de minga) | MLS de grupo (PCS), cifrado de sesión en la app wawa |
| **llimphi** | 85% | **Cerrar el deadlock click/scroll** (bug abierto) | Runtime sobre framebuffer Wawa, AA/texto en GPU directo |
| **nada** | 82% | Multi-ventana / split de editores | LSP rico (rename, code actions) |
| **tullpu** | 80% | Nodegraph visual (espera `llimphi-surface`), tiling | Proveedor IA ONNX real, PSD de salida, compositing GPU |
| **supay** | 78% | BSP-walking real (corrección de orden de render) | Audio vía takiy, renderer sobre HAL Wawa |
| **shuma** | 78% | Mouse en PTY, lockfile del daemon | Hover-drawer (bloqueado upstream en llimphi-ui) |
| **wawa** (host) | 72% | Toggles de módulos con efecto real, accent al theme global | SO_PEERCRED, migración a wawa-OS |
| **takiy** | 72% | (núcleo cerrado) pulir `takiy-midi` | Acoplar al bus chasqui, driver de audio Wawa |
| **nahual** | 68% | Visor PDF (falta rasterizador) + SVG + seek/scrub | AppBus out-of-process, meta-schema |
| **media** | 68% | **M1: sincronización A/V por PTS completa** (hoy parcial) | HW decode, streaming de red (RTMP/HLS) |
| **chasqui** | 62% | Persistencia del broker + transporte/discovery P2P | Routing tipo MQTT, ACLs + cifrado |
| **mirada** | 55% | Estabilidad del compositor + sesión/DM en hardware real | Compositor sobre framebuffer Wawa, greeter/DRM, multi-DPI |

## 03_ukupacha — RAÍZ

| App | % core | Falta para cerrar el core | Visión / stretch |
|---|---|---|---|
| **wawa** (kernel) | 88% | Capacidades 100% derivadas de la firma (§14.1.3) | Gaming (AOT cranelift, GPU passthrough, asset streaming), aarch64 |
| **agora** | 80% | Tabla de capacidades por bytecode hash | Las 17 apps de `APLICACIONES.md` (roadmap) |
| **minga** | 80% | `MingaPeer` genérico para escala (>100k nodos) | "Grafo de la Verdad" (GossipSub, reputación, +9 idiomas) |
| **arje** | 78% | Cleanup del socket daemon, RestartTracker en LocalEngine | aarch64/hardware real, dedup entre instantáneas |
| **wawa-explorer** | 78% | (read-only por diseño) sacar el process-monitor a su crate | Sync remota AoE inline, export de subárbol a `.tar` |
| **sandokan** | 60% | Cleanup socket, RunCard arbitraria, monitor Fase 4 (lado Wawa) | — |

## Lectura rápida

- **Prácticamente terminadas (≥85%):** pineal, khipu, tinkuy, wawa-kernel, ayni, pluma, rimay, llimphi, dominium.
- **Sólidas y usables (75–84%):** nada, cosmos, supay, tullpu, agora, minga, chaka, wawa-explorer, arje, shuma, puriy.
- **Funcionan, con hueco de core claro (60–74%):** takiy, wawa-host, nakui, nahual, media, iniy, sandokan, chasqui.
- **Cuello real (<60%):** mirada (depende de un compositor/DM estable sobre hardware).

### Patrones de lo que falta
- **Bugs/infra de Llimphi:** el deadlock click/scroll toca casi toda la línea de UIs; cerrarlo sube robustez transversal.
- **Integraciones cruzadas pendientes:** audio supay↔takiy, transporte chasqui, NAT traversal minga (lo arrastran ayni/chasqui/khipu).
- **De demo a producto:** nakui (verticales), media (M1 sync), iniy (pipeline e2e probado), mirada (sesión real).

### Nota sobre el recálculo
Las que más subieron vs el inventario viejo son las que él castigaba por *visión* y no por *función*:
**supay 35→78 · tinkuy 50→88 · iniy 40→65 · takiy 45→72 · sandokan ~35→60**.
`iniy` queda marcada ⚠️: se ve completa por piezas, pero el pipeline e2e no se verificó corriendo — no se sube sin verlo andar.
