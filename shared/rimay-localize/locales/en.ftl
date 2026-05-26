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
cosmos-header = cosmos · { $title } (mock · Asc { $asc }° MC { $mc }°)
cosmos-demo-title = Synthetic chart (demo)
cosmos-demo-subtitle = no real computation — just geometry

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
supay-controls-hint = WASD/← → move  ·  Ctrl fire  ·  Space use  ·  Tab map  ·  Esc menu  ·  F3 toggle view  ·  F12 quit
supay-stub-controls-hint = F3 toggles FB/3D  ·  F12 closes the window

# === shuma-shell ===
shuma-label-launcher = Launcher
shuma-label-command = Command
shuma-label-shell = Shell
shuma-label-matilda = Matilda
shuma-empty-main-incompat = Main module not compatible
shuma-empty-no-main = No Main module configured.
shuma-empty-no-main-hint = F12 opens the drawer with shell + monitors. Click on the command bar too.
shuma-empty-no-drawer-tabs = No tabs in the drawer.
shuma-empty-no-drawer-compat = This module cannot be a DrawerTab.
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
