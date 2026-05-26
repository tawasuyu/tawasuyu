# rimay-localize — qu-PE (Runasimi, variante sureña).
#
# Nota para el revisor humano: este catálogo es un PUNTO DE PARTIDA
# escrito por un desarrollador no nativo. Las formas elegidas siguen
# fuentes accesibles (AMLQ, Cusqueño escrito) pero piden corrección por
# alguien con dominio del idioma. Pluralización (sufijo -kuna), ergativo
# (-pa, -wan) y cortesía (-yki) son las áreas más sensibles.

# === acciones genéricas ===
save = Waqaychay
load = Apamuy
open = Kichay
close = Wisq'ay
cancel = Saqiy
confirm = Allinmi
yes = Arí
no = Manan
delete = Pichay
edit = Hukchay
new = Musuq

# === estado ===
play = Qallariy
pause = Samay
resume = Kutiy
stop = Sayachiy

# === menús ===
file = Qillqa
view = Qhaway
help = Yanapay
settings = Allichana
exit = Lluqsiy

# === niveles de mensaje ===
info = Willay
warning = Yuyaymanay
error = Pantay
success = Allinmi

# === interpolación ===
welcome-user = Allin hamusqaykim, { $name }.
items-count = { $count } imaymana.

# === dominium (chawpi pachapi pukllachiq) ===
dominium-status-running = ● purichkan
dominium-status-paused = ‖ samachkan
dominium-status-line = dominium · chawpi pacha   ·   wiñay { $epoch }   ·   thaski { $tick }
dominium-btn-pause = ‖  Samay
dominium-btn-resume = ▶  Kutiy
dominium-btn-reseed = ↺  Watiq taqraay
dominium-btn-create-concept = ✦  Yuyay ruway
dominium-btn-seed-pack = ✚  Taqra churay
dominium-btn-clear = ✖  Pichay
dominium-btn-save = 💾  Waqaychay
dominium-btn-load-saved = 📂  Waqaychasqa apamuy
dominium-btn-load-named = ✓ «{ $name }» apamuy
dominium-header-sim = [ PUKLLAY ]
dominium-header-conceptos = [ YUYAYKUNA ]
dominium-header-metricas = [ TUPUCHIQKUNA ]
dominium-header-editar = [ HUKCHAY ]
dominium-active-count = { $count } kawsachkan

# === cosmos (overlay módulos) ===
cosmos-btn-save-transit = 💾 Purichiqta qispi qillqaman waqaychay
cosmos-btn-save-progressed = 💾 Wiñasqata qispi qillqaman waqaychay
cosmos-btn-save-return = 💾 Kutiqta qispi qillqaman waqaychay

# === greeter (mirada login) ===
greeter-subtitle = sesionniykita qallariy
greeter-label-user = sutiyki
greeter-label-password = pakasqa rimay
greeter-placeholder-user = sutiykita churay
greeter-status-authenticating = qhawachkani…
greeter-error-empty-user = sutiyki churay

# === nakui (ERP shell) ===
nakui-header = Nakui · { $count } yanapakuq
nakui-sidebar-modules = Yanapakuqkuna ({ $count })
nakui-sidebar-menu = Akllana
nakui-empty-no-modules = Mana yanapakuq apamusqa
nakui-empty-pick-menu = Akllanata akllariy lateral barrapi
nakui-empty-pick-module = Yanapakuqta akllariy lateral barrapi
nakui-pending-edit = hukchay suyaykuchkan: meta-form Llimphi munakun
nakui-pending-render-detail = qhawachiy suyaykuchkan: meta-form Llimphi munakun
nakui-pending-render-dashboard = qhawachiy suyaykuchkan: dashboard Llimphi munakun

# === pluma (DAG hukchaq) ===
pluma-tone-valid = khuska
pluma-tone-pending = qhawana
pluma-tone-conflict = ch'aqwaypi

# === gioser-edit (qillqa hukchaq) ===
edit-status-find = maskay · Ctrl+G qatiq · Esc wisq'ay
edit-status-goto-def-waiting = goto-def · LSP suyaykuchkan…
edit-status-references-waiting = references · LSP suyaykuchkan…
edit-status-rename-input = sutichay · Enter ruway · Esc saqiy
edit-status-rename-waiting = sutichay → «{ $name }» · LSP suyaykuchkan…
edit-status-rename-error = sutichay · pantay { $path }: { $err }
edit-status-rename-done = sutichay · { $files } qillqa · { $bytes } bytes
edit-status-formatting-waiting = patachay · LSP suyaykuchkan…
edit-status-formatting-done = patachay · churasqa
edit-status-goto-def-at = goto-def · { $path }:{ $line }
edit-status-goto-def-error = goto-def · pantay kichaspa { $path }: { $err }
edit-status-saved = waqaychasqa · { $path }
edit-status-save-error = pantay waqaychaspa: { $err }
edit-header-hint = Ctrl+Shift+P akllana  ·  Ctrl+P qillqakuna  ·  Ctrl+Shift+F maskay
edit-status-position = Ln { $line }, Col { $col }  ·  { $lang }
