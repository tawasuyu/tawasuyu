# rimay-localize — en-US catalog

# === generic actions ===
save = Save
load = Load
open = Open
close = Close
cancel = Cancel
confirm = OK
yes = Yes
no = No
delete = Delete
edit = Edit
new = New

# === state ===
play = Play
pause = Pause
resume = Resume
stop = Stop

# === menus ===
file = File
view = View
help = Help
settings = Settings
exit = Exit

# === message levels ===
info = Info
warning = Warning
error = Error
success = Done

# === interpolation ===
welcome-user = Welcome, { $name }.
items-count = { $count } items.

# === dominium (mean-field simulator) ===
dominium-status-running = ● running
dominium-status-paused = ‖ paused
dominium-status-line = dominium · mean field   ·   epoch { $epoch }   ·   tick { $tick }
dominium-btn-pause = ‖  Pause
dominium-btn-resume = ▶  Resume
dominium-btn-reseed = ↺  Reseed
dominium-btn-create-concept = ✦  Create concept
dominium-btn-seed-pack = ✚  Seed pack
dominium-btn-clear = ✖  Clear
dominium-btn-save = 💾  Save
dominium-btn-load-saved = 📂  Load saved
dominium-btn-load-named = ✓ Load «{ $name }»
dominium-header-sim = [ SIM ]
dominium-header-conceptos = [ CONCEPTS ]
dominium-header-metricas = [ METRICS ]
dominium-header-editar = [ EDIT ]
dominium-active-count = { $count } active
dominium-stat-population = Population
dominium-stat-materia = Matter
dominium-stat-oro = Gold
dominium-stat-energia = Energy
dominium-stat-epoca = Epoch
dominium-stat-gini-energia = Gini energy
dominium-stat-edad-media = Mean age
dominium-stat-var-psi-orden = Var ψ order
dominium-stat-var-psi-miedo = Var ψ fear
dominium-stat-var-psi-curiosidad = Var ψ curiosity
dominium-stat-var-psi-corruptib = Var ψ corrupt.
dominium-action-mover = → move
dominium-action-extraer = → extract
dominium-action-sincronizar = → sync
dominium-action-intercambiar = → swap
dominium-action-replicar = → replicate
dominium-action-degradar = → degrade
dominium-slider-nombre = name
dominium-slider-radius = radius
dominium-slider-materia = matter
dominium-slider-psique = psyche
dominium-slider-poder = power
dominium-slider-oro = gold
dominium-label-hack = hack:

# === cosmos (overlay modules) ===
cosmos-btn-save-transit = 💾 Save transit as free chart
cosmos-btn-save-progressed = 💾 Save progressed as free chart
cosmos-btn-save-return = 💾 Save return as free chart
cosmos-header = cosmos · { $title } · Asc { $asc }° · MC { $mc }°
cosmos-demo-title = Sample chart (Lima)
cosmos-demo-subtitle = computed by cosmos-engine (VSOP2013)
cosmos-status = { $ms } ms · { $layers } layers · { $overlays } overlays · { $aspects } aspects
cosmos-status-error = error: { $err }
cosmos-overlay-transit = transit
cosmos-overlay-progression = progression
cosmos-overlay-solar-arc = solar arc
cosmos-overlay-uranian = uranian
cosmos-overlay-lots = lots
cosmos-overlay-fixed-stars = fixed stars
cosmos-overlay-midpoints = midpoints
cosmos-harmonic-label = harmonic
cosmos-empty = (empty)
cosmos-tile-carta = chart
cosmos-tile-modulos = modules
cosmos-tile-armonico = harmonic
cosmos-tile-cuerpos = bodies
cosmos-tile-aspectos = aspects
cosmos-tile-box-graph = aspectarian
cosmos-tile-cualidades = qualities
cosmos-elementos = elements
cosmos-modalidades = modalities
cosmos-polaridad = polarity
cosmos-elem-fuego = fire
cosmos-elem-tierra = earth
cosmos-elem-aire = air
cosmos-elem-agua = water
cosmos-mod-cardinal = cardinal
cosmos-mod-fijo = fixed
cosmos-mod-mutable = mutable
cosmos-pol-yang = yang
cosmos-pol-yin = yin
cosmos-tile-astrocarto = astrocartography
cosmos-astrocarto-leyenda = MC solid · IC dashed · Asc/Desc curves · • natal place
cosmos-tile-cartas = saved charts
cosmos-cartas-duplicar = + duplicate current
cosmos-cartas-vacio = (empty — duplicate current or drop JSONs in the dir)
cosmos-tile-corpus = corpus
cosmos-tile-lotes = lots
cosmos-tile-estrellas-fijas = fixed stars
cosmos-tile-puntos-medios = midpoints
cosmos-corpus-header = { $pasajes } passages · { $huecos } gaps · { $total } combos
cosmos-corpus-vacio = (no passages — write your corpus in cosmos-corpus/ejemplo.ron)
cosmos-tile-uraniano = uranian dial 90°
cosmos-tile-cross-transit = cross · transit
cosmos-tile-cross-progression = cross · progression
cosmos-tile-cross-solar-arc = cross · solar arc

# === wawa-explorer (Wawa image browser) ===
wawa-marker-via-aoe =   ·  via AoE
wawa-marker-searching =   ·  searching…
wawa-marker-fetch-failed =   ·  fetch failed
wawa-marker-not-in-image =   ·  (not in image)
wawa-iface-ok =   ·  AoE iface: { $name }
wawa-iface-err =   ·  AoE: no interface
wawa-header-error = wawa-explorer · error: { $err }
wawa-header = wawa-explorer · { $source }  ·  { $bytes } bytes  ·  v{ $version }  ·  cursor sector { $cursor }  ·  { $objects } objects{ $iface }
wawa-detail-empty = (select an object from the tree)
wawa-detail-title = object { $hash }  ·  { $bytes } bytes  ·  { $children } children{ $origen }
wawa-detail-title-missing = object { $hash }  ·  not present locally
wawa-detail-payload-header = payload (first 256 bytes):
wawa-detail-children-header = children:
wawa-detail-child-missing =   (not in image)
wawa-detail-searching-aoe-1 = searching the local network (AoE)…
wawa-detail-searching-aoe-2 = broadcast SolicitarObjeto, awaiting ProveedorObjeto with verified hash.
wawa-detail-fetch-error-1 = last AoE attempt failed:
wawa-detail-fetch-error-2 = you can retry with the button below.
wawa-detail-needs-fetch-1 = this object is referenced by a parent but does not live in the local image.
wawa-detail-needs-fetch-2 = you can request it from Wawa peers on the local network (AoE, iface `{ $iface }`).
wawa-detail-aoe-disabled-1 = this object is referenced by a parent but does not live in the local image.
wawa-detail-aoe-disabled-2 = AoE disabled: { $why }
wawa-detail-aoe-disabled-3 = pass `<iface>` as the second CLI argument or run with CAP_NET_RAW (`sudo setcap cap_net_raw=eip <binary>`).
wawa-btn-fetch = fetch from peers
wawa-btn-retry-fetch = retry fetch from peers
# main menu bar
wawa-menu-file = File
wawa-menu-reload = Reload image
wawa-menu-quit = Quit
wawa-menu-view = View
wawa-menu-fetch = Fetch node via AoE
wawa-menu-theme = Toggle theme
wawa-menu-help = Help
wawa-menu-about = About
# context menu on the selected node
wawa-ctx-select = Select
wawa-ctx-expand = Expand
wawa-ctx-collapse = Collapse
wawa-ctx-fetch = Fetch via AoE

# === minga-explorer (repo browser) ===
minga-header-loaded = Repo: { $path }  ·  reload { $ms } ms
minga-header-searching = Searching repo at { $path }…
minga-error-read = could not read repo { $path }: { $err }
minga-card-nodes-title = AST Nodes
minga-card-nodes-desc = code fragments parsed
minga-card-attestations-title = Attestations
minga-card-attestations-desc = Ed25519 signatures over nodes
minga-card-mst-title = MST Keys
minga-card-mst-desc = Merkle Search Tree entries
minga-empty = Waiting for first refresh…
minga-menu-file = File
minga-menu-view = View
minga-menu-help = Help
minga-menu-refresh = Refresh
minga-menu-quit = Quit
minga-menu-theme = Toggle theme
minga-menu-about = About
minga-menu-context-title = Repo

# === nakui-explorer (event log) ===
nakui-explorer-header = Log: { $path }  ·  { $entries } entries ({ $seeds } seeds, { $morphisms } morphisms)  ·  reload { $ms } ms
nakui-explorer-breakdown = breakdown: { $parts }

# === supay (doom) ===
supay-mode-real = REAL ENGINE
supay-mode-stub = STUB
supay-view-fb = view=FB (F3→3D)
supay-view-3d = view=3D (F3→FB)
supay-header = { $title }  ·  tick { $tick }  ·  { $mode }  ·  { $view }  ·  { $scene }
supay-stub-title = supay-doom-llimphi is running in STUB mode
supay-stub-step-1 = Clone doomgeneric
supay-stub-step-1-cmd =     cd 02_ruway/supay/supay-core/vendor && git clone https://github.com/ozkl/doomgeneric.git
supay-stub-step-2 = Drop the shareware WAD in the cwd
supay-stub-step-2-cmd =     curl -O https://distro.ibiblio.org/slitaz/sources/packages/d/doom1.wad
supay-stub-step-3 = Run it again
supay-stub-step-3-cmd =     cargo run -p supay-doom-llimphi --release
supay-stub-footer = doomgeneric (C) ticks at 35 Hz; the 320×200 ARGB framebuffer paints in aspect-fit.
supay-controls-hint = WASD · Ctrl fire · Space use · Tab map · F3 view · F4 cross · F5 vig · F6 HUD · F7 shadows · F8 muzzle · F9 occl · F10 mobj-lit · F11 rim · F12 quit
supay-stub-controls-hint = F3 toggles FB/3D  ·  F12 closes the window

# === shuma-shell ===
shuma-label-launcher = Launcher
shuma-label-command = Command
shuma-label-shell = Shell
shuma-label-matilda = Matilda
shuma-label-canvas = Canvas
shuma-empty-main-incompat = Main module not compatible
shuma-empty-no-tabs = No tabs configured.
shuma-empty-no-tabs-compat = This module cannot be a tab.
shuma-empty-no-data-linux = no data (not Linux?)
shuma-empty-no-data = no data
shuma-stat-samples = samples: { $have } / { $total }

# === nahual (viewers) ===
nahual-image-unsupported = unsupported format (only PNG/JPEG in this build)

# === greeter (mirada login) ===
greeter-subtitle = sign in
greeter-label-user = username
greeter-label-password = password
greeter-placeholder-user = enter your username
greeter-status-authenticating = verifying…
greeter-error-empty-user = enter a username

# === nakui (ERP shell) ===
nakui-header = Nakui · { $count } module(s)
nakui-sidebar-modules = Modules ({ $count })
nakui-sidebar-menu = Menu
nakui-empty-no-modules = No modules loaded
nakui-empty-pick-module = Pick a module in the sidebar
nakui-empty-pick-menu = Pick a menu in the sidebar
nakui-pending-edit = edit pending: requires Llimphi meta-form
nakui-pending-render-detail = render pending: requires Llimphi meta-form
nakui-pending-render-dashboard = render pending: requires Llimphi dashboard

# === pluma (DAG editor) ===
pluma-tone-valid = coherent
pluma-tone-pending = pending
pluma-tone-conflict = conflict

# === gioser-edit (code editor) ===
edit-status-find = find · Ctrl+G next · Esc closes
edit-status-goto-def-waiting = goto-def · waiting for LSP…
edit-status-references-waiting = references · waiting for LSP…
edit-status-rename-input = rename · Enter applies · Esc cancels
edit-status-rename-waiting = rename → «{ $name }» · waiting for LSP…
edit-status-rename-error = rename · error in { $path }: { $err }
edit-status-rename-done = rename · { $files } files · { $bytes } bytes
edit-status-formatting-waiting = formatting · waiting for LSP…
edit-status-formatting-done = formatting · applied
edit-status-goto-def-at = goto-def · { $path }:{ $line }
edit-status-goto-def-error = goto-def · error opening { $path }: { $err }
edit-status-saved = saved · { $path }
edit-status-save-error = save error: { $err }
edit-header-hint = Ctrl+Shift+P palette  ·  Ctrl+P files  ·  Ctrl+Shift+F search
edit-status-position = Ln { $line }, Col { $col }  ·  { $lang }

# === chasqui-explorer (monads) ===
chasqui-header = Engine '{ $engine }'  ·  { $count } monad(s)  ·  socket: { $socket } ({ $src }){ $watching }
chasqui-header-watching =   ·  watching: { $name }
chasqui-header-searching = Searching chasqui daemon via brahman-broker…
chasqui-field-id = id: { $id }
chasqui-field-watching = watching: { $name }
chasqui-field-keywords = keywords: { $keywords }
chasqui-field-path = path: { $path }
chasqui-field-model = model: { $name }

# === wawa-panel (wawa OS control panel) ===
wawa-panel-title = wawa · control panel
wawa-panel-cat-appearance = Appearance
wawa-panel-cat-language = Language
wawa-panel-cat-apps = Applications
wawa-panel-cat-monitor = Monitor
wawa-panel-cat-modules = Modules
wawa-panel-cat-about = About
wawa-panel-section-appearance-hint = Theme variant and accent.
wawa-panel-section-language-hint = System language and clock format.
wawa-panel-section-apps-hint = Launch wawa native apps.
wawa-panel-section-monitor-hint = Live system state.
wawa-panel-section-modules-hint = Enable or disable OS modules.
wawa-panel-section-about-hint = Operating system information.
wawa-panel-label-variant = Variant
wawa-panel-label-accent = Accent
wawa-panel-label-language = Language
wawa-panel-label-clock = Clock
wawa-panel-variant-dark = Dark
wawa-panel-variant-light = Light
wawa-panel-variant-aurora = Aurora
wawa-panel-variant-sunset = Sunset
wawa-panel-clock-24h = 24 h
wawa-panel-clock-12h = 12 h
wawa-panel-stat-time = Time
wawa-panel-stat-uptime = Uptime
wawa-panel-stat-mem = Memory
wawa-panel-stat-load = Load
wawa-panel-stat-host = Host
wawa-panel-stat-kernel = Kernel
wawa-panel-action-launch = Launch
wawa-panel-action-save = Save config
wawa-panel-action-reset = Reset
wawa-panel-saved = Configuration saved to { $path }
wawa-panel-reset = Configuration reset to defaults
wawa-panel-menu-file = File
wawa-panel-menu-view = View
wawa-panel-menu-help = Help
wawa-panel-menu-quit = Quit
wawa-panel-status-hint = ↑↓ navigate  ·  Enter activate  ·  Ctrl+S save  ·  Esc exit
wawa-panel-about-name = System
wawa-panel-about-version = Version
wawa-panel-about-kernel = Kernel
wawa-panel-about-toolkit = Toolkit
wawa-panel-about-blurb = wawa is the operating system of the gioser suite. arje kernel and llimphi apps over a minimal userland.
wawa-panel-mod-mirada = mirada · wayland compositor
wawa-panel-mod-shuma = shuma · packaging and releases
wawa-panel-mod-chasqui = chasqui · mail and messaging
wawa-panel-mod-akasha = akasha · update channel
wawa-panel-mod-minga = minga · p2p storage
wawa-panel-mod-agora = agora · public square
wawa-panel-mod-on = on
wawa-panel-mod-off = off

# === mirada-asistente ===
# Llimphi app that translates natural language into mirada-ctl commands
# by consulting an LLM. The AI proposes; the human confirms before executing.
asistente-title = carmen · assistant
asistente-sub = tell me what you want to do; the assistant proposes, you confirm.
asistente-placeholder = what do you want to do? (Enter to ask, Esc to clear)
asistente-banner-no-llm = LLM unavailable: { $motivo }
asistente-status-pensando = thinking…
asistente-boton-ejecutar = Run
asistente-boton-descartar = Discard
asistente-ejecutado-ok = ✓ { $accion } executed
asistente-ejecutado-fallo = ✗ { $accion } failed
asistente-error-transporte = transport: { $motivo }
asistente-error-sin-llm = LLM not initialized
asistente-error-sin-json = response without JSON: { $crudo }
asistente-error-accion-vacia = proposal without action: { $crudo }
asistente-error-json-invalido = unrecognized JSON: { $crudo }
asistente-error-spawn = spawn failed: { $err } (is mirada-ctl in PATH?)
asistente-cero-salida = (no output)
asistente-codigo-salida = exit code { $codigo }
asistente-error-accion-desconocida = LLM proposed an unknown action: { $accion }
