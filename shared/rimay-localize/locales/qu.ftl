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
cosmos-header = cosmos · { $title } (mock · Asc { $asc }° MC { $mc }°)
cosmos-demo-title = Carta ruwasqa (demo)
cosmos-demo-subtitle = mana yupasqa — geometría sapanlla

# === wawa-explorer (Wawa imagen qhawana) ===
wawa-marker-via-aoe =   ·  AoE-pi
wawa-marker-searching =   ·  maskachkan…
wawa-marker-fetch-failed =   ·  fetch pantasqa
wawa-marker-not-in-image =   ·  (mana imagenpi)
wawa-iface-ok =   ·  AoE iface: { $name }
wawa-iface-err =   ·  AoE: mana interfaz
wawa-header-error = wawa-explorer · pantay: { $err }
wawa-header = wawa-explorer · { $source }  ·  { $bytes } bytes  ·  v{ $version }  ·  cursor sector { $cursor }  ·  { $objects } imaymana{ $iface }
wawa-detail-empty = (huk imaymanata akllariy tree-pi)
wawa-detail-title = imaymana { $hash }  ·  { $bytes } bytes  ·  { $children } wawa{ $origen }
wawa-detail-title-missing = imaymana { $hash }  ·  mana kaypi
wawa-detail-payload-header = payload (ñawpaq 256 bytes):
wawa-detail-children-header = wawakuna:
wawa-detail-child-missing =   (mana imagenpi)
wawa-detail-searching-aoe-1 = local red AoE-pi maskachkan…
wawa-detail-searching-aoe-2 = broadcast SolicitarObjeto, suyay ProveedorObjeto verified hash-niyuq.
wawa-detail-fetch-error-1 = AoE intento pantasqa:
wawa-detail-fetch-error-2 = kay botón qhipata watiq maskayta atinki.
wawa-detail-needs-fetch-1 = kay imaymana huk tayta-pi nisqa, ichaqa mana local imagen-pi kawsachkan.
wawa-detail-needs-fetch-2 = local red Wawa peer-kuna-mantapis mañakuyta atinki (AoE, iface `{ $iface }`).
wawa-detail-aoe-disabled-1 = kay imaymana huk tayta-pi nisqa, ichaqa mana local imagen-pi kawsachkan.
wawa-detail-aoe-disabled-2 = AoE wisq'asqa: { $why }
wawa-detail-aoe-disabled-3 = CLI iskaynin parlachi-pi `<iface>` churay icha CAP_NET_RAW-wan kachay (`sudo setcap cap_net_raw=eip <binario>`).
wawa-btn-fetch = peer-kuna-manta apamuy
wawa-btn-retry-fetch = peer-kuna-manta watiq apamuy

# === minga-explorer (repo qhawana) ===
minga-header-loaded = Repo: { $path }  ·  watiq apamuy { $ms } ms
minga-header-searching = { $path }-pi repo maskachkan…
minga-error-read = { $path } repo mana ñawinchayta atinichu: { $err }
minga-card-nodes-title = AST Yuyay
minga-card-nodes-desc = código-manta parsesqa fragments
minga-card-attestations-title = Firmasqakuna
minga-card-attestations-desc = nodos hawapi Ed25519 firmakuna
minga-card-mst-title = MST Llaves
minga-card-mst-desc = Merkle Search Tree-pi yaykuq
minga-empty = Ñawpaq refresh suyachkan…

# === nakui-explorer (event log) ===
nakui-explorer-header = Log: { $path }  ·  { $entries } yaykuq ({ $seeds } seeds, { $morphisms } morphisms)  ·  watiq apamuy { $ms } ms
nakui-explorer-breakdown = rakiy: { $parts }

# === supay (doom) ===
supay-mode-real = MOTOR PAQARIQ
supay-mode-stub = STUB
supay-view-fb = qhaway=FB (F3→3D)
supay-view-3d = qhaway=3D (F3→FB)
supay-header = { $title }  ·  thaski { $tick }  ·  { $mode }  ·  { $view }  ·  { $scene }
supay-stub-title = supay-doom-llimphi STUB modo-pi purichkan

# === shuma-shell ===
shuma-label-launcher = Launcher
shuma-label-command = Kamachiq
shuma-label-shell = Shell
shuma-label-matilda = Matilda
shuma-empty-main-incompat = Main yanapakuq mana atinmanchu
shuma-empty-no-main = Mana Main yanapakuq churasqa.
shuma-empty-no-main-hint = F12 drawer-ta kichan shell + qhawanakunawan. Command bar-pi click-pis.
shuma-empty-no-drawer-tabs = Mana tabs drawer-pi.
shuma-empty-no-drawer-compat = Kay yanapakuq mana DrawerTab kayta atinmanchu.
shuma-empty-no-data-linux = mana willay (¿manachu Linux?)
shuma-empty-no-data = mana willay
shuma-stat-samples = qhawasqakuna: { $have } / { $total }

# === nahual (qhawanakuna) ===
nahual-image-unsupported = mana atisqa formato (kay build-pi PNG/JPEG sapanlla)

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
