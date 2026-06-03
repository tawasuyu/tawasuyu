# APPS.md — catálogo de `cargo run` para probar todo gioser

Referencia exhaustiva de cada app, binario y demo ejecutable del workspace.
Generado recorriendo `Cargo.toml` + `src/main.rs` + `[[bin]]` + `examples/*.rs`.

Convenciones:
- **Apps gráficas (Llimphi)**: agregá `--release` siempre — wgpu/vello van lentos en debug.
- Pasar argumentos: `cargo run -p <crate> -- --help`.
- `03_ukupacha/wawa` (kernel SASOS) está **excluido** del workspace raíz; se construye aparte (`cd 03_ukupacha/wawa && cargo +nightly run -p boot -Z bindeps`). Aquí sólo van los crates host-side de wawa que sí compilan en el workspace global.

---

## 00_unanchay — PERCIBIR

### chaka (transpilador COBOL → Rust)
```bash
cargo run -p chaka-app                 # CLI: lexer → parser → IR → codegen
cargo run -p chaka-app-llimphi --release   # GUI del transpilador
```

### khipu (notas al olvido)
```bash
cargo run -p khipu-app --release       # cuaderno sobre Llimphi
cargo run -p khipu-app --example demo_cli
```

### pineal (librería de gráficos multi-dimensión)
```bash
cargo run -p pineal-demo --release           # cartesiano multi-serie
cargo run -p pineal-bars-demo --release      # barras
cargo run -p pineal-contour-demo --release   # contornos (8 isolíneas + heatmap)
cargo run -p pineal-financial-demo --release # velas OHLC (random walk)
cargo run -p pineal-flow-demo --release      # Sankey (presupuesto familiar)
cargo run -p pineal-galeria-demo --release   # galería estática completa
cargo run -p pineal-gpu-demo --release       # starfield 3D warp (GPU directo)
cargo run -p pineal-heatmap-demo --release   # campo 2D con onda viajera
cargo run -p pineal-hexbin-demo --release    # 5.000 puntos → hexágonos
cargo run -p pineal-mesh-demo --release      # grafo de 24 nodos relajando
cargo run -p pineal-phosphor-demo --release  # osciloscopio CRT con estela
cargo run -p pineal-polar-demo --release     # pie/donut + radar
cargo run -p pineal-stream-demo --release    # osciloscopio sintético
cargo run -p pineal-treemap-demo --release   # treemap squarified (12 tiles)
```

### pluma (editor multilienzo + notebook + deck)
```bash
cargo run -p pluma-app --release             # editor multilienzo (splitters)
cargo run -p pluma-deck-app --release        # presentaciones espaciales (tipo Prezi)
cargo run -p pluma-notebook-app --release    # notebook reproducible
cargo run -p pluma-notebook-llimphi --release # visor read-only de notebook
```
Demos de editor:
```bash
cargo run -p pluma-editor-llimphi --example multilienzo_demo --release
cargo run -p pluma-editor-llimphi --example multilienzo_completo_demo --release
cargo run -p pluma-editor-llimphi --example multilienzo_dinamico_demo --release
cargo run -p pluma-editor-llimphi --example multilienzo_llm_demo --release
cargo run -p pluma-editor-llimphi --example multilienzo_store_demo --release
cargo run -p pluma-editor-llimphi --example cuerpo_ide_demo --release
cargo run -p pluma-editor-llimphi --example editor_unico_demo --release
cargo run -p pluma-editor-llimphi --example zona_transform_demo --release
```
Demos de deck (presentaciones):
```bash
cargo run -p pluma-deck-recorrido-llimphi --example recorrido_demo --release
cargo run -p pluma-deck-recorrido-llimphi --example recorrido_editor_demo --release
cargo run -p pluma-deck-recorrido-llimphi --example recorrido_imagen_demo --release
cargo run -p pluma-deck-recorrido-llimphi --example recorrido_md_demo --release
```
Demos de notebook / LLM:
```bash
cargo run -p pluma-notebook-graph-llimphi --example notebook_graph_demo --release
cargo run -p pluma-notebook-graph-llimphi --example notebook_graph_dominium_demo --release
cargo run -p pluma-notebook-kernel-llm --example notebook_llm_demo --release
cargo run -p pluma-llm-gemini --example smoke
```

### puriy (motor de render HTML/CSS)
```bash
cargo run -p puriy-app --release       # navegador HTML/CSS
cargo run -p puriy-engine --example load_example_com
cargo run -p puriy-js --example inspect_wasm
```

### rimay (embeddings)
```bash
cargo run -p rimay-verbo-daemon-bin -- --provider fastembed  # daemon de embeddings
cargo run -p rimay-verbo-daemon-bin -- --provider mock --dim 384
```

---

## 01_yachay — CONOCER

### cosmos (astrometría + astrología)
```bash
cargo run -p cosmos-app-llimphi --release   # shell astronómico/astrológico
cargo run -p cosmos-cli                      # cliente socket Tahuantinsuyu
cargo run -p cosmos-server                   # servidor HTTP cosmobiología
```
Herramientas CLI:
```bash
cargo run -p cosmos-catalog --bin forge
cargo run -p cosmos-catalog --bin query-catalog
cargo run -p cosmos-ephemeris --bin vsop2013-gen
cargo run -p cosmos-ephemeris --bin elpmpp02-gen
cargo run -p cosmos-pointing --bin pointing
cargo run -p cosmos-validation --bin precision-report
cargo run -p cosmos-validation --bin sidereal-check
cargo run -p cosmos-validation --bin houses-check
cargo run -p cosmos-validation --bin topocentric-check
cargo run -p cosmos-validation --bin lunar-check
cargo run -p cosmos-validation --bin stars-check
cargo run -p cosmos-validation --bin altaz-check
cargo run -p cosmos-validation --bin risetrans-check
cargo run -p cosmos-validation --bin eclipses-check
cargo run -p cosmos-validation --bin asteroids-check
cargo run -p cosmos-validation --bin local-eclipses-check
```
Demos:
```bash
cargo run -p cosmos-astrology --example natal_chart
cargo run -p cosmos-canvas-llimphi --example dense_starfield --release
cargo run -p cosmos-catalog --example cone_search
cargo run -p cosmos-coords --example coordinate_transforms
cargo run -p cosmos-coords --example eop_basics
cargo run -p cosmos-coords --example eop_update
cargo run -p cosmos-coords --example galactic_ecliptic
cargo run -p cosmos-coords --example solar_lunar
cargo run -p cosmos-eclipses --example next_eclipses_demo
cargo run -p cosmos-engine --example wheel
cargo run -p cosmos-leo --example iss_pass_demo
cargo run -p cosmos-notebook-kernel --example notebook_cosmos_demo
cargo run -p cosmos-render --example catalog
cargo run -p cosmos-rise-set --example rise_set_lima_demo
cargo run -p cosmos-skywatch --example skywatch_lima_demo
cargo run -p cosmos-sundial --example sundial_lima_demo
cargo run -p cosmos-tides --example tides_callao_demo
cargo run -p cosmos-transits --example next_transits_demo
```

### dominium (simulador de física)
```bash
cargo run -p dominium-app-llimphi --release  # simulador con ventana
cargo run -p dominium-cli                     # simulación sin ventana (stats)
cargo run -p dominium-canvas-llimphi --example canvas_demo --release
cargo run -p dominium-notebook-kernel --example notebook_dominium_demo
```

### iniy (laboratorio de creencias semánticas)
```bash
cargo run -p iniy-cli
cargo run -p iniy-explorer-llimphi --release  # visualizar corpus
cargo run -p iniy-server                       # API HTTP de consultas
cargo run -p iniy-wiki                          # importador de dumps Wikipedia
```

### nakui (ERP modular con scripts Rhai)
```bash
cargo run -p nakui-core --bin nakui            # binario principal
cargo run -p nakui-core --bin demo
cargo run -p nakui-core --bin inventory_demo
cargo run -p nakui-core --bin sales_demo
cargo run -p nakui-core --bin crm_demo
cargo run -p nakui-explorer-llimphi --release  # visor de log de eventos
cargo run -p nakui-sheet-llimphi --release     # planilla tipo Excel
cargo run -p nakui-sheet --bin sheet_demo
cargo run -p nakui-ui-llimphi --release        # shell metainterfaz nakui
```

### tinkuy (sistema de partículas)
```bash
cargo run -p tinkuy-sim                         # demo end-to-end
cargo run -p tinkuy-llimphi --example tinkuy_demo --release
```

---

## 02_ruway — HACER

### ayni (red de reciprocidad / chat P2P)
```bash
cargo run -p ayni-cli
cargo run -p ayni-llimphi --release
```

### chasqui (presencia y mensajería sobre cards)
```bash
cargo run -p chasqui-core --bin chasqui                  # broker/daemon
cargo run -p chasqui-broker-explorer-llimphi --release   # explorador del broker
cargo run -p chasqui-explorer-llimphi --release          # panel de descubrimiento
cargo run -p chasqui-nous-mock                            # provider embeddings determinista
cargo run -p chasqui-nous-real                            # provider embeddings con LLM
```
Demos de cards:
```bash
cargo run -p card-admin --example brahman-status
cargo run -p card-handshake --example probe
cargo run -p card-handshake --example subscriber
cargo run -p card-sidecar --example presence
cargo run -p card-sidecar --example presence-conscious
```

### llimphi (motor gráfico soberano)
```bash
cargo run -p llimphi-gallery --release         # demo maestro de componentes
cargo run -p llimphi-widget-gallery --release  # todos los widgets en una app
cargo run -p llimphi-gpu-bench --release       # benchmark GPU
```
Demos de widgets:
```bash
cargo run -p llimphi-hal --example clear_screen --release
cargo run -p llimphi-icons --example app_icons_gallery --release
cargo run -p llimphi-layout --example layout_panels --release
cargo run -p llimphi-raster --example gpu_million_points --release
cargo run -p llimphi-raster --example render_node --release
cargo run -p llimphi-raster --example spike_gpu_directo --release
cargo run -p llimphi-text --example hello_text --release
cargo run -p llimphi-ui --example counter --release
cargo run -p llimphi-ui --example editor --release
cargo run -p llimphi-ui --example gpu_paint_demo --release
cargo run -p llimphi-widget-button --example button_demo --release
cargo run -p llimphi-widget-nodegraph --example nodegraph_demo --release
cargo run -p llimphi-widget-panes --example panes_demo --release
cargo run -p llimphi-widget-slider --example slider_demo --release
cargo run -p llimphi-widget-splitter --example splitter_demo --release
cargo run -p llimphi-widget-tabs --example tabs_demo --release
cargo run -p llimphi-widget-theme-switcher --example theme_switcher_demo --release
cargo run -p llimphi-widget-tiled --example tiled_demo --release
cargo run -p llimphi-widget-tree --example tree_demo --release
cargo run -p llimphi-widget-wawa-mark --example wawa_mark_demo --release
cargo run -p llimphi-workspace --example workspace_demo --release
```

### media (audio/video)
```bash
cargo run -p media-app --release            # reproductor A/V
cargo run -p media-recorder-app --release   # grabador de pantalla (AV1+Opus)
cargo run -p media-app --example analyze
cargo run -p media-encode-av1 --example gradient
cargo run -p media-source-av1 --example av1_decode
cargo run -p media-source-capture --example grabar_pantalla
cargo run -p media-source-capture --example grabar_pantalla_audio
```

### mirada (compositor + shell de escritorio)
```bash
cargo run -p mirada-app-llimphi --release    # ventana del compositor (Cerebro)
cargo run -p mirada-asistente-llimphi --release  # asistente conversacional
cargo run -p mirada-compositor               # daemon compositor (Cuerpo)
cargo run -p mirada-ctl                       # control CLI del compositor
cargo run -p mirada-greeter --release         # greeter (display manager)
cargo run -p mirada-launcher --release        # lanzador de apps
cargo run -p mirada-portal                    # backend portal XDG
cargo run -p asistente-puente                 # puente Linux (scaffolding)
cargo run -p mirada-body --example headless
cargo run -p mirada-brain --example headless-ctl
cargo run -p mirada-brain --example keymap-default
```

### nahual (visor universal de archivos)
```bash
cargo run -p nahual-gallery-llimphi --release  # galería tipo gThumb/FastStone
cargo run -p nahual-shell-llimphi --release    # file manager
cargo run -p nahual-image-viewer-llimphi --example image_viewer_demo --release
cargo run -p nahual-video-viewer-llimphi --example video_viewer_demo --release
cargo run -p nahual-audio-viewer-llimphi --example audio_viewer_demo --release
cargo run -p nahual-map-viewer-llimphi --example probar --release
```

### nada (editor de archivos rápido)
```bash
cargo run -p nada --release            # file tree + editor sobre archivos reales
```

### paloma (cliente de correo)
```bash
cargo run -p paloma-app --release
cargo run -p paloma-llimphi --example buzon_demo --release
```

### pata (inspector del framework del sistema)
```bash
cargo run -p pata-config               # inspector de configuración
cargo run -p pata-llimphi --release    # frontend del framework
```

### raymi (calendario y contactos)
```bash
cargo run -p raymi-app --release
cargo run -p raymi-llimphi --example agenda_demo --release
```

### shuma (shell + ejecución de comandos)
```bash
cargo run -p shuma-cli                  # admin CLI
cargo run -p shuma-daemon               # daemon runtime
cargo run -p shuma-gateway              # adaptador HTTP/JSON
cargo run -p shuma-shell-llimphi --release  # shell UI
cargo run -p matilda                    # admin de servidor baremetal
```

### supay (motor de juego / port de Doom)
```bash
cargo run -p supay-app-llimphi --release    # Fase 0.5 (frontend)
cargo run -p supay-doom-llimphi --release   # port de Doom (Fase 1)
cargo run -p supay-doom-llimphi --example dump_frame
```

### takiy (sintetizador MIDI)
```bash
cargo run -p takiy-app-llimphi --release    # piano roll + player
cargo run -p takiy-synth --example demo
cargo run -p takiy-app-llimphi --example smoke
```

### tullpu (gráficos raster / pintura)
```bash
cargo run -p tullpu-app-llimphi --release   # app de pintura
cargo run -p pixel-verbo-daemon-bin         # daemon provider de pixels
```

### uya (videollamada soberana)
```bash
cargo run -p uya-llimphi --release   # rejilla de caras + cámara/micrófono/colgar
cargo run -p uya-cli                  # nodo headless (transporte + captura)
```
Dos extremos en local (uno escucha, otro conecta):
```bash
UYA_NOMBRE=Alicia UYA_ESCUCHAR=127.0.0.1:7800 cargo run -p uya-llimphi --release
UYA_NOMBRE=Beto UYA_ESCUCHAR=127.0.0.1:7801 \
  UYA_CONECTAR=127.0.0.1:7800 cargo run -p uya-llimphi --release
```

### wawa (componentes host-side del SO)
```bash
cargo run -p wawa-panel-llimphi --release   # panel de control de Wawa
cargo run -p wawactl                         # CLI de config de Wawa
```

---

## 03_ukupacha — RAÍZ

### agora (consenso + gossip)
```bash
cargo run -p agora-app --release       # UI de agora
cargo run -p agora-cli                  # operaciones de shell
cargo run -p agora-channel --example forjar_propuesta_mudanza_demo
cargo run -p agora-gossip --example two_node_sync
cargo run -p agora-net-brahman --example convergencia_minga
```

### arje (sistema de init)
```bash
cargo run -p arje-absorb               # traductor de config de otros inits
cargo run -p arje-getty-stub           # getty mínimo
cargo run -p arje-installer            # CLI de instalación
cargo run -p arje-net-bring-up         # levanta enlaces de red
cargo run -p arje-packager             # empaquetador de seed cards
cargo run -p arje-zero                  # PID 1 (primer Ente)
cargo run -p arje-echo                  # Ente echo (provider mínimo)
cargo run -p arje-card-llimphi --release  # card de escritorio
```
Capas de compatibilidad (shims systemd):
```bash
cargo run -p arje-compat --bin arje-binfmt-compat
cargo run -p arje-compat --bin arje-hostnamed-compat
cargo run -p arje-compat --bin arje-journald-compat
cargo run -p arje-compat --bin arje-journalctl
cargo run -p arje-compat --bin arje-localed-compat
cargo run -p arje-compat --bin arje-logind-compat
cargo run -p arje-compat --bin arje-machined-compat
cargo run -p arje-compat --bin arje-notify-compat
cargo run -p arje-compat --bin arje-polkit-compat
cargo run -p arje-compat --bin arje-policy-provider
cargo run -p arje-compat --bin arje-resolved-compat
cargo run -p arje-compat --bin arje-systemd1-compat
cargo run -p arje-compat --bin arje-timedated-compat
cargo run -p arje-compat --bin arje-timer-compat
cargo run -p arje-compat --bin arje-tmpfiles-compat
cargo run -p arje-brain --example brainctl
cargo run -p arje-bus --example busctl
```

### minga (VCS distribuido + almacenamiento P2P)
```bash
cargo run -p minga-cli
cargo run -p minga-explorer-llimphi --release  # dashboard de repos
```

### sandokan (orquestador de procesos)
```bash
cargo run -p sandokan-app                        # CLI orquestador
cargo run -p sandokan-monitor-llimphi --release  # monitor de procesos
```

### wawa (host-side: explorador de imágenes)
```bash
cargo run -p wawa-explorer-llimphi --release   # explorador de imágenes Wawa
cargo run -p wawa-explorer-aoe --example servir_release
cargo run -p wawa-explorer-aoe --example solicitar
cargo run -p wawa-explorer-core --example dump
```

> El SO bare-metal completo (`03_ukupacha/wawa/wawa-kernel` + apps WASM) va aparte:
> ```bash
> cd 03_ukupacha/wawa && cargo +nightly run -p boot -Z bindeps   # forja imagen UEFI + QEMU
> ```

---

## shared — transversales

```bash
cargo run -p auth-core --example auth-probe
cargo run -p card-wit --example brahman-wit-info
cargo run -p foreign-psd --example psd_a_png
cargo run -p launcher-llimphi --example launcher_demo --release
cargo run -p rimay-localize --example showcase
```

---

## Notas

- **Recuento aproximado**: ~120 binarios + ~155 demos `--example`.
- Apps Llimphi sin `--release` arrancan pero a pocos FPS; usá siempre `--release` para UI.
- `pluma-llm-*` y kernels LLM caen a backend Mock sin `ANTHROPIC_API_KEY`/`GEMINI_API_KEY`/etc. — los demos arrancan igual.
- El smoke test mínimo del workspace sigue siendo `cargo check --workspace`.
