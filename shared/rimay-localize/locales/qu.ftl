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
cosmos-header = cosmos · { $title } · Asc { $asc }° · MC { $mc }°
cosmos-demo-title = Qhawanapaq qillqa (Lima)
cosmos-demo-subtitle = cosmos-engine yupan (VSOP2013)
cosmos-status = { $ms } ms · { $layers } qatakuna · { $overlays } overlays · { $aspects } aspectos
cosmos-status-error = pantasqa: { $err }
cosmos-overlay-transit = puriq
cosmos-overlay-progression = wiñay
cosmos-overlay-solar-arc = inti arco
cosmos-overlay-uranian = uraniano
cosmos-overlay-lots = lote
cosmos-overlay-fixed-stars = qulluy
cosmos-overlay-midpoints = chawpi
cosmos-harmonic-label = armónico
cosmos-empty = (manaña)
cosmos-tile-carta = qillqa
cosmos-tile-modulos = módulos
cosmos-tile-armonico = armónico
cosmos-tile-cuerpos = ukhukuna
cosmos-tile-aspectos = aspectos
cosmos-tile-uraniano = uraniano 90° muyu
cosmos-tile-cross-transit = cross · puriq
cosmos-tile-cross-progression = cross · wiñay
cosmos-tile-cross-solar-arc = cross · inti arco

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
supay-stub-step-1 = doomgeneric-ta apamuy
supay-stub-step-1-cmd =     cd 02_ruway/supay/supay-core/vendor && git clone https://github.com/ozkl/doomgeneric.git
supay-stub-step-2 = WAD shareware-ta cwd-man apamuy
supay-stub-step-2-cmd =     curl -O https://distro.ibiblio.org/slitaz/sources/packages/d/doom1.wad
supay-stub-step-3 = Watiq kachay
supay-stub-step-3-cmd =     cargo run -p supay-doom-llimphi --release
supay-stub-footer = doomgeneric (C) 35 Hz-pi puriy; framebuffer 320×200 ARGB aspect-fit-wan llimpisqa.
supay-controls-hint = WASD/← → kuyuy  ·  Ctrl tuksiy  ·  Space kichay  ·  Tab mapa  ·  Esc akllana  ·  F3 qhaway tikray  ·  F12 lluqsiy
supay-stub-controls-hint = F3 FB/3D tikray  ·  F12 wisq'ay

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

# === wawa-panel (wawa SO panilninkuna) ===
wawa-panel-title = wawa · kamachiy panil
wawa-panel-cat-appearance = Rikch'aynin
wawa-panel-cat-language = Simi
wawa-panel-cat-apps = Llamk'anakuna
wawa-panel-cat-monitor = Qhaway
wawa-panel-cat-modules = T'aqakuna
wawa-panel-cat-about = Imaynan
wawa-panel-section-appearance-hint = Llinphi rikch'ay, tinki ima.
wawa-panel-section-language-hint = Sistemaq simin, pacha rikch'ay ima.
wawa-panel-section-apps-hint = wawaq llamk'ananta kichay.
wawa-panel-section-monitor-hint = Kunan sistemaq munay kawsaynin.
wawa-panel-section-modules-hint = SOq t'aqankunata churay otaq qichuy.
wawa-panel-section-about-hint = Sistemaq willaynin.
wawa-panel-label-variant = Rikch'ay
wawa-panel-label-accent = Tinki
wawa-panel-label-language = Simi
wawa-panel-label-clock = Pacha
wawa-panel-variant-dark = Llanthu
wawa-panel-variant-light = K'anchay
wawa-panel-variant-aurora = Aurora
wawa-panel-variant-sunset = Inti haykuy
wawa-panel-clock-24h = 24 h
wawa-panel-clock-12h = 12 h
wawa-panel-stat-time = Pacha
wawa-panel-stat-uptime = Sayasqa pacha
wawa-panel-stat-mem = Yuyay
wawa-panel-stat-load = Q'ipi
wawa-panel-stat-host = Wasiq
wawa-panel-stat-kernel = Sunqu
wawa-panel-action-launch = Kichay
wawa-panel-action-save = Waqaychay
wawa-panel-action-reset = Kaqmanta churay
wawa-panel-saved = Kamachiy waqaychasqa { $path } nisqapi
wawa-panel-reset = Kamachiy kaqmanta churasqa
wawa-panel-status-hint = ↑↓ puriy  ·  Enter ruway  ·  Ctrl+S waqaychay  ·  Esc lluqsiy
wawa-panel-about-name = Sistema
wawa-panel-about-version = Mit'a
wawa-panel-about-kernel = Sunqu
wawa-panel-about-toolkit = Llamk'ana qillqana
wawa-panel-about-blurb = wawa kaqmi gioser suiteq sistemaynin. arje sunqu, llimphi llamk'anakuna, huch'uy userland patapi.
wawa-panel-mod-mirada = mirada · wayland kamachiq
wawa-panel-mod-shuma = shuma · q'ipichay, willay ima
wawa-panel-mod-chasqui = chasqui · willasqa, chasqui ima
wawa-panel-mod-akasha = akasha · musuqyachiy ñan
wawa-panel-mod-minga = minga · p2p waqaychana
wawa-panel-mod-agora = agora · llaqta plaza
wawa-panel-mod-on = kasqa
wawa-panel-mod-off = wañusqa
