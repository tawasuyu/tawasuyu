# rimay-localize — catálogo es-PE
# Convención: IDs en kebab-case, ASCII, en inglés (estables); traducción
# en este archivo. Comentarios (#) describen contexto cuando el ID no
# basta.

# === acciones genéricas ===
save = Guardar
load = Cargar
open = Abrir
close = Cerrar
cancel = Cancelar
confirm = Aceptar
yes = Sí
no = No
delete = Eliminar
edit = Editar
new = Nuevo

# === estado ===
play = Reproducir
pause = Pausar
resume = Reanudar
stop = Detener

# === menús ===
file = Archivo
view = Vista
help = Ayuda
settings = Configuración
exit = Salir

# === niveles de mensaje ===
info = Información
warning = Advertencia
error = Error
success = Listo

# === interpolación ===
welcome-user = Bienvenido, { $name }.
items-count = { $count } elementos.

# === dominium (simulador de campo medio) ===
dominium-status-running = ● corriendo
dominium-status-paused = ‖ en pausa
dominium-status-line = dominium · campo medio   ·   época { $epoch }   ·   tick { $tick }
dominium-btn-pause = ‖  Pausar
dominium-btn-resume = ▶  Reanudar
dominium-btn-reseed = ↺  Re-sembrar
dominium-btn-create-concept = ✦  Crear concepto
dominium-btn-seed-pack = ✚  Sembrar pack
dominium-btn-clear = ✖  Limpiar
dominium-btn-save = 💾  Guardar
dominium-btn-load-saved = 📂  Cargar guardado
dominium-btn-load-named = ✓ Cargar «{ $name }»
dominium-header-sim = [ SIM ]
dominium-header-conceptos = [ CONCEPTOS ]
dominium-header-metricas = [ MÉTRICAS ]
dominium-header-editar = [ EDITAR ]
dominium-active-count = { $count } activos
dominium-stat-population = Población
dominium-stat-materia = Materia
dominium-stat-oro = Oro
dominium-stat-energia = Energía
dominium-stat-epoca = Época
dominium-stat-gini-energia = Gini energía
dominium-stat-edad-media = Edad media
dominium-stat-var-psi-orden = Var ψ orden
dominium-stat-var-psi-miedo = Var ψ miedo
dominium-stat-var-psi-curiosidad = Var ψ curiosidad
dominium-stat-var-psi-corruptib = Var ψ corruptib.
dominium-action-mover = → mover
dominium-action-extraer = → extraer
dominium-action-sincronizar = → sincronizar
dominium-action-intercambiar = → intercambiar
dominium-action-replicar = → replicar
dominium-action-degradar = → degradar
dominium-slider-nombre = nombre
dominium-slider-radius = radius
dominium-slider-materia = materia
dominium-slider-psique = psique
dominium-slider-poder = poder
dominium-slider-oro = oro
dominium-label-hack = hack:

# === cosmos (módulos overlay) ===
cosmos-btn-save-transit = 💾 Guardar tránsito como carta libre
cosmos-btn-save-progressed = 💾 Guardar progresada como carta libre
cosmos-btn-save-return = 💾 Guardar retorno como carta libre
cosmos-header = cosmos · { $title } · Asc { $asc }° · MC { $mc }°
cosmos-demo-title = Carta de muestra (Lima)
cosmos-demo-subtitle = computada por cosmos-engine (VSOP2013)
cosmos-status = { $ms } ms · { $layers } capas · { $overlays } overlays · { $aspects } aspectos
cosmos-status-error = error: { $err }
cosmos-overlay-transit = tránsito
cosmos-overlay-progression = progresión
cosmos-overlay-solar-arc = arco solar
cosmos-overlay-uranian = uraniano
cosmos-overlay-lots = lotes
cosmos-overlay-fixed-stars = est. fijas
cosmos-overlay-midpoints = puntos medios
cosmos-harmonic-label = armónico
cosmos-empty = (vacío)
cosmos-tile-carta = carta
cosmos-tile-modulos = módulos
cosmos-tile-armonico = armónico
cosmos-tile-cuerpos = cuerpos
cosmos-tile-aspectos = aspectos
cosmos-tile-box-graph = aspectarian
cosmos-tile-cualidades = cualidades
cosmos-elementos = elementos
cosmos-modalidades = modalidades
cosmos-polaridad = polaridad
cosmos-elem-fuego = fuego
cosmos-elem-tierra = tierra
cosmos-elem-aire = aire
cosmos-elem-agua = agua
cosmos-mod-cardinal = cardinal
cosmos-mod-fijo = fijo
cosmos-mod-mutable = mutable
cosmos-pol-yang = yang
cosmos-pol-yin = yin
cosmos-tile-astrocarto = astrocartografía
cosmos-astrocarto-leyenda = MC sólido · IC punteado · Asc/Desc curvas · • lugar natal
cosmos-tile-cartas = cartas guardadas
cosmos-cartas-duplicar = + duplicar la actual
cosmos-cartas-vacio = (vacío — duplicá la actual o copiá JSONs al dir)
cosmos-tile-corpus = corpus
cosmos-tile-lotes = lotes
cosmos-tile-estrellas-fijas = estrellas fijas
cosmos-tile-puntos-medios = puntos medios
cosmos-corpus-header = { $pasajes } pasajes · { $huecos } huecos · { $total } combinaciones
cosmos-corpus-vacio = (sin pasajes — escribí el corpus en cosmos-corpus/ejemplo.ron)
cosmos-tile-uraniano = dial uraniano 90°
cosmos-tile-cross-transit = cross · tránsito
cosmos-tile-cross-progression = cross · progresión
cosmos-tile-cross-solar-arc = cross · arco solar

# === wawa-explorer (Wawa image browser) ===
wawa-marker-via-aoe =   ·  via AoE
wawa-marker-searching =   ·  buscando…
wawa-marker-fetch-failed =   ·  fetch falló
wawa-marker-not-in-image =   ·  (no en imagen)
wawa-iface-ok =   ·  AoE iface: { $name }
wawa-iface-err =   ·  AoE: sin interfaz
wawa-header-error = wawa-explorer · error: { $err }
wawa-header = wawa-explorer · { $source }  ·  { $bytes } bytes  ·  v{ $version }  ·  cursor sector { $cursor }  ·  { $objects } objetos{ $iface }
wawa-detail-empty = (seleccioná un objeto del tree)
wawa-detail-title = objeto { $hash }  ·  { $bytes } bytes  ·  { $children } hijos{ $origen }
wawa-detail-title-missing = objeto { $hash }  ·  no presente localmente
wawa-detail-payload-header = payload (primeros 256 bytes):
wawa-detail-children-header = hijos:
wawa-detail-child-missing =   (no en imagen)
wawa-detail-searching-aoe-1 = buscando en la red local (AoE)…
wawa-detail-searching-aoe-2 = broadcast SolicitarObjeto, espera ProveedorObjeto con hash verificado.
wawa-detail-fetch-error-1 = último intento de AoE falló:
wawa-detail-fetch-error-2 = podés reintentar con el botón debajo.
wawa-detail-needs-fetch-1 = este objeto está referenciado por un padre pero no vive en la imagen local.
wawa-detail-needs-fetch-2 = podés pedirlo a peers Wawa de la red local (AoE, iface `{ $iface }`).
wawa-detail-aoe-disabled-1 = este objeto está referenciado por un padre pero no vive en la imagen local.
wawa-detail-aoe-disabled-2 = AoE deshabilitado: { $why }
wawa-detail-aoe-disabled-3 = pasá `<iface>` como segundo argumento de CLI o ejecutá con CAP_NET_RAW (`sudo setcap cap_net_raw=eip <binario>`).
wawa-btn-fetch = fetch from peers
wawa-btn-retry-fetch = reintentar fetch from peers

# === minga-explorer (repo browser) ===
minga-header-loaded = Repo: { $path }  ·  reload { $ms } ms
minga-header-searching = Buscando repo en { $path }…
minga-error-read = no pude leer repo { $path }: { $err }
minga-card-nodes-title = Nodos AST
minga-card-nodes-desc = fragments parseados del código
minga-card-attestations-title = Atestaciones
minga-card-attestations-desc = firmas Ed25519 sobre los nodos
minga-card-mst-title = Claves MST
minga-card-mst-desc = entradas del Merkle Search Tree
minga-empty = Esperando primer refresh…

# === nakui-explorer (event log) ===
nakui-explorer-header = Log: { $path }  ·  { $entries } entries ({ $seeds } seeds, { $morphisms } morphisms)  ·  reload { $ms } ms
nakui-explorer-breakdown = breakdown: { $parts }

# === supay (doom) ===
supay-mode-real = ENGINE REAL
supay-mode-stub = STUB
supay-view-fb = view=FB (F3→3D)
supay-view-3d = view=3D (F3→FB)
supay-header = { $title }  ·  tick { $tick }  ·  { $mode }  ·  { $view }  ·  { $scene }
supay-stub-title = supay-doom-llimphi corre en modo STUB
supay-stub-step-1 = Cloná doomgeneric
supay-stub-step-1-cmd =     cd 02_ruway/supay/supay-core/vendor && git clone https://github.com/ozkl/doomgeneric.git
supay-stub-step-2 = Bajá el WAD shareware al cwd
supay-stub-step-2-cmd =     curl -O https://distro.ibiblio.org/slitaz/sources/packages/d/doom1.wad
supay-stub-step-3 = Volvé a correr
supay-stub-step-3-cmd =     cargo run -p supay-doom-llimphi --release
supay-stub-footer = doomgeneric (C) avanza a 35 Hz; el framebuffer 320×200 ARGB se pinta en aspect-fit.
supay-controls-hint = WASD/← → mover  ·  Ctrl disparar  ·  Space usar  ·  Tab mapa  ·  Esc menú  ·  F3 alterna vista  ·  F12 salir
supay-stub-controls-hint = F3 alterna FB/3D  ·  F12 cierra la ventana

# === shuma-shell ===
shuma-label-launcher = Launcher
shuma-label-command = Command
shuma-label-shell = Shell
shuma-label-matilda = Matilda
shuma-empty-main-incompat = Módulo Main no compatible
shuma-empty-no-main = Sin módulo Main configurado.
shuma-empty-no-main-hint = F12 abre el drawer con shell + monitores. Click en la command bar también.
shuma-empty-no-drawer-tabs = Sin tabs en el drawer.
shuma-empty-no-drawer-compat = Este módulo no puede ser DrawerTab.
shuma-empty-no-data-linux = sin datos (¿no es Linux?)
shuma-empty-no-data = sin datos
shuma-stat-samples = muestras: { $have } / { $total }

# === nahual (visores) ===
nahual-image-unsupported = formato no soportado (sólo PNG/JPEG en esta build)

# === greeter (mirada login) ===
greeter-subtitle = iniciá tu sesión
greeter-label-user = usuario
greeter-label-password = contraseña
greeter-placeholder-user = ingresá tu usuario
greeter-status-authenticating = verificando…
greeter-error-empty-user = ingresá un usuario

# === nakui (ERP shell) ===
nakui-header = Nakui · { $count } módulo(s)
nakui-sidebar-modules = Módulos ({ $count })
nakui-sidebar-menu = Menú
nakui-empty-no-modules = Sin módulos cargados
nakui-empty-pick-menu = Elegí un menú en la barra lateral
nakui-empty-pick-module = Elegí un módulo en la barra lateral
nakui-pending-edit = edición pendiente: requiere meta-form Llimphi
nakui-pending-render-detail = render pendiente: requiere meta-form Llimphi
nakui-pending-render-dashboard = render pendiente: requiere dashboard Llimphi

# === pluma (editor DAG) ===
pluma-tone-valid = coherente
pluma-tone-pending = por evaluar
pluma-tone-conflict = en conflicto

# === gioser-edit (editor de código) ===
edit-status-find = find · Ctrl+G siguiente · Esc cierra
edit-status-goto-def-waiting = goto-def · esperando LSP…
edit-status-references-waiting = references · esperando LSP…
edit-status-rename-input = rename · Enter aplica · Esc cancela
edit-status-rename-waiting = rename → «{ $name }» · esperando LSP…
edit-status-rename-error = rename · error en { $path }: { $err }
edit-status-rename-done = rename · { $files } archivos · { $bytes } bytes
edit-status-formatting-waiting = formatting · esperando LSP…
edit-status-formatting-done = formatting · aplicado
edit-status-goto-def-at = goto-def · { $path }:{ $line }
edit-status-goto-def-error = goto-def · error abriendo { $path }: { $err }
edit-status-saved = guardado · { $path }
edit-status-save-error = error guardando: { $err }
edit-header-hint = Ctrl+Shift+P palette  ·  Ctrl+P files  ·  Ctrl+Shift+F search
edit-status-position = Ln { $line }, Col { $col }  ·  { $lang }

# === chasqui-explorer (mónadas) ===
chasqui-header = Engine '{ $engine }'  ·  { $count } mónada(s)  ·  socket: { $socket } ({ $src }){ $watching }
chasqui-header-watching =   ·  watching: { $name }
chasqui-header-searching = Buscando daemon chasqui vía brahman-broker…
chasqui-field-id = id: { $id }
chasqui-field-watching = watching: { $name }
chasqui-field-keywords = keywords: { $keywords }
chasqui-field-path = path: { $path }
chasqui-field-model = model: { $name }

# === wawa-panel (panel de control del SO wawa) ===
wawa-panel-title = wawa · panel de control
wawa-panel-cat-appearance = Apariencia
wawa-panel-cat-language = Idioma
wawa-panel-cat-apps = Aplicaciones
wawa-panel-cat-monitor = Monitor
wawa-panel-cat-modules = Módulos
wawa-panel-cat-about = Acerca de
wawa-panel-section-appearance-hint = Variante del tema y acento.
wawa-panel-section-language-hint = Idioma del sistema y formato de hora.
wawa-panel-section-apps-hint = Lanzá las apps nativas de wawa.
wawa-panel-section-monitor-hint = Estado vivo del sistema.
wawa-panel-section-modules-hint = Activar o desactivar piezas del SO.
wawa-panel-section-about-hint = Información del sistema operativo.
wawa-panel-label-variant = Variante
wawa-panel-label-accent = Acento
wawa-panel-label-language = Idioma
wawa-panel-label-clock = Reloj
wawa-panel-variant-dark = Oscuro
wawa-panel-variant-light = Claro
wawa-panel-variant-aurora = Aurora
wawa-panel-variant-sunset = Sunset
wawa-panel-clock-24h = 24 h
wawa-panel-clock-12h = 12 h
wawa-panel-stat-time = Hora
wawa-panel-stat-uptime = Uptime
wawa-panel-stat-mem = Memoria
wawa-panel-stat-load = Carga
wawa-panel-stat-host = Host
wawa-panel-stat-kernel = Kernel
wawa-panel-action-launch = Lanzar
wawa-panel-action-save = Guardar config
wawa-panel-action-reset = Restablecer
wawa-panel-saved = Configuración guardada en { $path }
wawa-panel-reset = Configuración restablecida a valores por defecto
wawa-panel-status-hint = ↑↓ navegar  ·  Enter activar  ·  Ctrl+S guardar  ·  Esc salir
wawa-panel-about-name = Sistema
wawa-panel-about-version = Versión
wawa-panel-about-kernel = Núcleo
wawa-panel-about-toolkit = Toolkit
wawa-panel-about-blurb = wawa es el sistema operativo de la suite gioser. El kernel arje y las apps llimphi sobre un userland mínimo.
wawa-panel-mod-mirada = mirada · compositor wayland
wawa-panel-mod-shuma = shuma · empaquetado y release
wawa-panel-mod-chasqui = chasqui · correo y mensajería
wawa-panel-mod-akasha = akasha · canal de actualizaciones
wawa-panel-mod-minga = minga · almacenamiento p2p
wawa-panel-mod-agora = agora · plaza pública
wawa-panel-mod-on = encendido
wawa-panel-mod-off = apagado

# === mirada-asistente ===
# App Llimphi que traduce lenguaje natural a comandos de mirada-ctl
# consultando un LLM. La IA propone, el humano confirma antes de ejecutar.
asistente-title = carmen · asistente
asistente-sub = describí lo que querés hacer; el asistente propone, vos confirmás.
asistente-placeholder = ¿qué querés hacer? (Enter para preguntar, Esc para limpiar)
asistente-banner-no-llm = LLM no disponible: { $motivo }
asistente-status-pensando = pensando…
asistente-boton-ejecutar = Ejecutar
asistente-boton-descartar = Descartar
asistente-ejecutado-ok = ✓ { $accion } ejecutado
asistente-ejecutado-fallo = ✗ { $accion } falló
asistente-error-transporte = transporte: { $motivo }
asistente-error-sin-llm = LLM no inicializado
asistente-error-sin-json = respuesta sin JSON: { $crudo }
asistente-error-accion-vacia = propuesta sin accion: { $crudo }
asistente-error-json-invalido = JSON no reconocido: { $crudo }
asistente-error-spawn = spawn falló: { $err } (¿está mirada-ctl en PATH?)
asistente-cero-salida = (sin salida)
asistente-codigo-salida = código { $codigo }
asistente-error-accion-desconocida = el LLM propuso una accion desconocida: { $accion }
