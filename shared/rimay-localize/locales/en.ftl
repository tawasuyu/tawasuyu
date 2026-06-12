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

# === common chrome (reusable across all Llimphi apps) ===
# Shared menu/action labels. An app only mints its own (`<app>-*`) IDs
# for text not covered here.
search = Search
language = Language
undo = Undo
redo = Redo
cut = Cut
copy = Copy
paste = Paste
select-all = Select All
open-dots = Open…
save-as = Save As…
close-tab = Close Tab
find-in-file = Find in File
find-in-project = Find in Project
symbols = Symbols
goto-definition = Go to Definition
terminal = Terminal
command-palette = Command Palette
minimap = Minimap
cycle-theme = Cycle Theme
editing = Editing
about = About
refresh = Refresh
reconnect = Reconnect

# === nada (file editor) ===
nada-tagline = a sovereign editor on Llimphi
nada-settings-appearance = Appearance
nada-settings-theme = Theme
nada-settings-editor = Editor
nada-settings-fmt-on-save = Format on save
nada-settings-demo-diag = Demo diagnostics
nada-settings-lsp = Language server

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
shuma-label-monitors = Monitors
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

# === tawasuyu-edit (code editor) ===
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
wawa-panel-about-blurb = wawa is the operating system of the tawasuyu suite. arje kernel and llimphi apps over a minimal userland.
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


# === ayni-llimphi ===
ayni-menu-admitir = Admit selected
ayni-menu-atestar = Attest selected
ayni-menu-expulsar = Expel selected
ayni-menu-enviar-msg = Send message
ayni-menu-adjuntar = Attach file…
ayni-menu-acuse = Read receipt
ayni-menu-cifrado = E2EE Encryption
ayni-menu-recibos = Read receipts
ayni-menu-comandos-barra = Slash commands
ayni-label-gente-miembros = PEOPLE — members
ayni-label-otros-vistos = others seen
ayni-label-acciones = actions
ayni-label-elige-alguien = pick someone above
ayni-btn-admitir = admit
ayni-btn-atestar = attest
ayni-btn-expulsar = expel
ayni-btn-acuse = receipt
ayni-label-confianza = trust (hops)
ayni-label-sin-atestaciones = — no attestations —
ayni-label-sin-mensajes = — no messages. Type below (or /help for commands). —
ayni-compose-placeholder = type a message, or /adjuntar <path>, /atestar <hex> …
ayni-btn-enviar = send

# === chaka-app-llimphi ===
chaka-menu-run = Run
chaka-menu-run-pipeline = Run pipeline
chaka-tab-output = Output
chaka-tab-rust = Generated Rust
chaka-tab-diag = Diagnostics
chaka-btn-run = Run
chaka-corpus-empty = empty corpus
chaka-corpus-header = CORPUS
chaka-editor-placeholder = select a program from the corpus
chaka-no-file = no file
chaka-banner-open-corpus = open a program from the corpus on the left
chaka-banner-step-limit = shadow ⚠ step limit reached (infinite loop?)
chaka-banner-pipeline-error = the pipeline failed — see the «Diag» tab for details
chaka-status-no-open-file = no open file to save
chaka-about-text = chaka · COBOL → Rust transpiler · pipeline lex→parse→ir→codegen→shadow

# === chasqui-explorer-llimphi ===
chasqui-explorer-ctx-detail = View detail
chasqui-explorer-monad-label = Monad
chasqui-explorer-monad-stats = { $count } files · ent { $entropy } · { $lens }

# === media-app ===
media-settings-tab-audio = Audio
media-settings-tab-video = Video
media-settings-tab-playback = Playback
media-settings-tab-bars = Bars
media-settings-tab-controls = Controls
media-audio-volume = Volume
media-audio-eq = Equalizer
media-audio-normalization = Normalization
media-audio-lufs-target = LUFS Target
media-audio-downmix = Stereo downmix
media-video-color = Color
media-video-enable = Enable
media-video-brightness = Brightness
media-video-contrast = Contrast
media-video-gamma = Gamma
media-video-saturation = Saturation
media-video-hue = Hue
media-video-orientation = Orientation
media-video-rotation = Rotation
media-video-rotate-cw = rotate 90°
media-video-flip-h = Flip H
media-video-flip-v = Flip V
media-action-reset = reset
media-action-cycle = cycle
media-playback-playlist = Playlist
media-playback-resume = Resume on open
media-playback-repeat = Repeat
media-playback-shuffle = Shuffle
media-playback-subtitles = Subtitles
media-playback-autoload-sidecar = Auto-load sidecar
media-playback-sub-delay = Delay (ms)
media-playback-font-size = Font size
media-playback-behavior = Behavior
media-playback-crossfade = Crossfade (s)
media-controls-header = Controls (keyboard)
media-controls-hint = Edit controles.ron and press F5 to reassign keys. The visual shortcut editor is coming later.
media-bars-header = Control bars — click an item to remove it
media-bars-bar-label = Bar
media-bars-remove-bar = − remove bar
media-bars-add-bar = + new bar
media-bars-add-items-to = Add items to:
media-settings-footer = Saved to config.ron · Esc closes · in Bars: click an item to remove, ‹ › reorder
media-playlist-header = Playlist — click a track to jump
media-playlist-empty = No playlist.
media-win-config-title = Settings — media
media-win-playlist-title = Playlist — media
media-help-title = media · shortcuts
media-help-group-playback = Playback
media-help-toggle = Show/hide this help
media-help-close = Close help
media-help-reload = Hot-reload controles.ron
media-menu-capture-frame = Capture frame
media-menu-record = Record / stop
media-menu-reload-controls = Reload controls
media-menu-playback = Playback
media-menu-play-pause = Play / pause
media-menu-seek-back = Seek backward
media-menu-seek-fwd = Seek forward
media-menu-prev-track = Previous track
media-menu-next-track = Next track
media-menu-volume-up = Volume up
media-menu-volume-down = Volume down
media-menu-playlist = Playlist
media-menu-visualizers = Audio visualizers
media-menu-shortcuts-help = Shortcut help
media-ctx-stop-record = Stop recording
media-ctx-record-audio = Record audio

# === mirada-app-llimphi ===
mirada-menu-open-window = Open window
mirada-menu-open-output = Open monitor
mirada-menu-close-focused = Close focused
mirada-menu-window = Window
mirada-win-promote = Promote to master
mirada-win-float = Float / anchor
mirada-win-fullscreen = Full screen
mirada-win-scratchpad = Send to scratchpad
mirada-win-label-fallback = window
mirada-layout-cycle = Cycle layout
mirada-layout-master-stack = Master + stack
mirada-layout-monocle = Monocle
mirada-layout-grid = Grid
mirada-layout-columns = Columns
mirada-layout-rows = Rows
mirada-layout-centered = Centered master
mirada-layout-spiral = Spiral
mirada-layout-shrink = Shrink master
mirada-layout-grow = Grow master
mirada-output-next = Next monitor
mirada-view-overview = Spatial view
mirada-status-body-connected = Body connected
mirada-status-simulation = simulation — no Body
mirada-status-keymap-reloaded = keymap reloaded
mirada-status-keymap-invalid = invalid keymap
mirada-label-layout = layout
mirada-label-focus = focus
mirada-label-output = output
mirada-label-workspace = workspace
mirada-canvas-empty-hint = empty workspace — press n to open a window
mirada-win-kind-fullscreen = · full screen ·
mirada-win-kind-floating = · floating window ·
mirada-win-kind-surface = · body surface ·

# === mirada-greeter ===
mirada-greeter-menu-session = Session
mirada-greeter-session-submit = Log in
mirada-greeter-session-goto-user = Go to username
mirada-greeter-session-goto-pass = Go to password
mirada-greeter-label-desktop = Desktop
mirada-greeter-btn-submit = Log in
mirada-greeter-btn-submitting = Logging in…
mirada-greeter-hint-nav = ↑/↓: desktop · Enter: log in
mirada-greeter-hint-console = Ctrl+Alt+F1…F12: console · Ctrl+Alt+⌫: exit

# === nakui-explorer-llimphi ===
nakui-explorer-menu-refresh-log = Refresh log
nakui-explorer-ctx-view-detail = View detail
nakui-explorer-ctx-refresh-log = Refresh log
nakui-explorer-ctx-entry-fallback = Entry

# === nakui-sheet-llimphi ===
nakui-sheet-ctx-clear = Clear
nakui-sheet-fmt-number = Format: Number
nakui-sheet-fmt-currency = Format: Currency $
nakui-sheet-fmt-percent = Format: Percentage
nakui-sheet-fmt-general = Format: General
nakui-sheet-freeze-here = Freeze Panes Here
nakui-sheet-unfreeze = Unfreeze Panes
nakui-sheet-pivot = Pivot Table…
nakui-sheet-menu-cell-cut = Cut Cell
nakui-sheet-menu-cell-copy = Copy Cell
nakui-sheet-menu-cell-paste = Paste Cell
nakui-sheet-menu-cell-clear = Clear Cell
nakui-sheet-menu-bar-cut = Cut Text
nakui-sheet-menu-bar-copy = Copy Text
nakui-sheet-menu-bar-paste = Paste Text
nakui-sheet-menu-bar-select-all = Select All (text)
nakui-sheet-menu-import-csv = Import CSV
nakui-sheet-menu-export-csv = Export CSV
nakui-sheet-menu-about = About Nakui Sheet
nakui-sheet-formula-placeholder = enter formula or value
nakui-sheet-pivot-title = Pivot Table
nakui-sheet-pivot-close = ✕ Esc
nakui-sheet-pivot-group-by = Group by «
nakui-sheet-pivot-over = over
nakui-sheet-pivot-with-header = w/header
nakui-sheet-pivot-no-header = no header
nakui-sheet-pivot-more-groups = groups
nakui-sheet-pivot-total = TOTAL
nakui-sheet-pivot-groups = groups
nakui-sheet-pivot-rows = rows
nakui-sheet-pivot-hint = A function · G group · V value · H header · Esc close

# === paloma-llimphi ===
paloma-status-init = paloma · not synced
paloma-status-search-semantic = semantic search (rimay): pending — using exact
paloma-status-view-rich = rich HTML via puriy: pending (plain text for now)
paloma-status-no-recipient = cannot send: missing a valid recipient
paloma-status-sent = sent
paloma-status-sent-signed = sent · signed (Ed25519)
paloma-placeholder-search = Search… ( / )
paloma-btn-compose = ✎ Compose
paloma-nav-calendar = Calendar
paloma-nav-contacts = Contacts
paloma-nav-soon = soon
paloma-empty-threads = Empty inbox
paloma-empty-search = no matches
paloma-search-exact = Exact
paloma-search-semantic = Semantic
paloma-no-subject = (no subject)
paloma-placeholder-read = Select a thread to read it
paloma-btn-reply = Reply
paloma-btn-forward = Forward
paloma-btn-star = Star
paloma-btn-starred = Starred
paloma-btn-mark-unread = Mark as unread
paloma-btn-mark-read = Mark as read
paloma-btn-view-rich = View rich HTML
paloma-msg-to-label = to
paloma-sig-verified = signed
paloma-sig-invalid = invalid signature
paloma-compose-new = New message
paloma-compose-reply-title = Reply
paloma-compose-placeholder-to = To: name <email@domain>
paloma-compose-placeholder-cc = Cc: (optional)
paloma-compose-placeholder-subject = Subject
paloma-compose-placeholder-body = Write your message…
paloma-compose-sign = Sign (Ed25519)
paloma-compose-send = Send

# === pluma-notebook-llimphi ===
pluma-notebook-fit-all = Fit All
pluma-notebook-center = Center
pluma-notebook-zoom-reset = Zoom 100%

# === raymi-llimphi ===
raymi-tab-calendar = Calendar
raymi-tab-contacts = Contacts
raymi-view-month = Month
raymi-view-week = Week
raymi-view-day = Day
raymi-btn-new-event = ＋ Event
raymi-btn-today = Today
raymi-btn-new-contact = ＋ Contact
raymi-no-events = no events
raymi-all-day = all day
raymi-no-contacts = no contacts
raymi-search-contact-placeholder = 🔍 Search contact…
raymi-select-contact-hint = Select a contact
raymi-title-edit-event = Edit event
raymi-title-new-event = New event
raymi-title-edit-contact = Edit contact
raymi-title-new-contact = New contact
raymi-change-cycle = change
raymi-field-summary = Subject
raymi-field-all-day = All day
raymi-field-apply-to = Apply to
raymi-field-calendar = Calendar
raymi-field-date = Date
raymi-field-start = Start
raymi-field-end = End
raymi-field-location = Location
raymi-ph-location = Location (optional)
raymi-field-description = Description
raymi-ph-description = Notes (optional)
raymi-field-attendees = Attendees
raymi-ph-invitee = Name <email> · Enter
raymi-field-repeat = Repeat
raymi-field-every = Every
raymi-field-days = Days
raymi-field-ends = Ends
raymi-field-name = Name
raymi-field-emails = Emails
raymi-field-phones = Phones
raymi-field-org = Organization
raymi-field-note = Note
raymi-ph-full-name = Full name
raymi-ph-emails = email@domain, other@…
raymi-ph-phones = +1 555…, …
raymi-ph-org = Company (optional)
raymi-ph-note = Note (optional)
raymi-scope-series = All events in series
raymi-scope-this-only = This event only
raymi-scope-this-and-future = This and following events
raymi-repeat-none = Does not repeat
raymi-repeat-daily = Daily
raymi-repeat-weekly = Weekly
raymi-repeat-monthly = Monthly
raymi-repeat-yearly = Yearly
raymi-unit-days = day(s)
raymi-unit-weeks = week(s)
raymi-unit-months = month(s)
raymi-unit-years = year(s)
raymi-end-never = Never ends
raymi-end-count = After N times
raymi-end-until = Until date
raymi-status-no-calendars = no calendars available to create an event
raymi-status-no-books = no address books available to create a contact
raymi-status-invalid-datetime = invalid date or time (use YYYY-MM-DD and HH:MM)
raymi-status-contact-needs-name = contact requires a name

# === shuma-shell-llimphi ===
shuma-shell-clear-input = Clear input
shuma-shell-clear-screen = Clear screen
shuma-shell-cancel-cmd = Cancel command
shuma-shell-about = About shuma
shuma-layouts = Layouts…

# === supay-app-llimphi ===
supay-hud-health = HEALTH
supay-hud-ammo = AMMO
supay-hud-target = TARGET
supay-action-fire = Fire
supay-action-reset = Restart game
supay-menu-play = Play
supay-status-game-over = game over
supay-status-victory = victory
supay-status-dead = DEAD
supay-hint-space-restart = SPACE to restart

# === wawa-panel-llimphi ===
wawa-panel-status-config-updated = ↻ config updated from bus
wawa-panel-ctx-refresh-monitor = Refresh monitor
wawa-panel-autosave-ok = ↻ applied
