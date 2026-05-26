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

# === cosmos (overlay modules) ===
cosmos-btn-save-transit = 💾 Save transit as free chart
cosmos-btn-save-progressed = 💾 Save progressed as free chart
cosmos-btn-save-return = 💾 Save return as free chart

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
