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
dominium-stat-population = Runa hunt'ay
dominium-stat-materia = Materia
dominium-stat-oro = Quri
dominium-stat-energia = Kallpa
dominium-stat-epoca = Wiñay
dominium-stat-gini-energia = Gini kallpa
dominium-stat-edad-media = Watayuq chawpi
dominium-stat-var-psi-orden = Var ψ kamachiy
dominium-stat-var-psi-miedo = Var ψ manchakuy
dominium-stat-var-psi-curiosidad = Var ψ tapukuy
dominium-stat-var-psi-corruptib = Var ψ ismuriy
dominium-action-mover = → kuyuy
dominium-action-extraer = → hurquy
dominium-action-sincronizar = → tinkichiy
dominium-action-intercambiar = → chhalaway
dominium-action-replicar = → kikinchay
dominium-action-degradar = → uray
dominium-slider-nombre = suti
dominium-slider-radius = mukmu
dominium-slider-materia = materia
dominium-slider-psique = nuna
dominium-slider-poder = atiy
dominium-slider-oro = quri
dominium-label-hack = hack:

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

# === chasqui-explorer (mónadas) ===
chasqui-header = Engine '{ $engine }'  ·  { $count } mónada  ·  socket: { $socket } ({ $src }){ $watching }
chasqui-header-watching =   ·  qhawachkan: { $name }
chasqui-header-searching = Chasqui daemonta maskaspa brahman-brokerwan…
chasqui-field-id = id: { $id }
chasqui-field-watching = qhawachkan: { $name }
chasqui-field-keywords = rimaykuna: { $keywords }
chasqui-field-path = ñan: { $path }
chasqui-field-model = modelo: { $name }
