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
