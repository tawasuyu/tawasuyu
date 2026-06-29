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

# === chrome común (reutilizable por todas las apps Llimphi) ===
# Etiquetas de menú/acción compartidas. Una app sólo crea IDs propios
# (`<app>-*`) para texto que no aparezca acá.
search = Buscar
language = Idioma
undo = Deshacer
redo = Rehacer
cut = Cortar
copy = Copiar
paste = Pegar
select-all = Seleccionar todo
open-dots = Abrir…
save-as = Guardar como…
close-tab = Cerrar pestaña
find-in-file = Buscar en archivo
find-in-project = Buscar en proyecto
symbols = Símbolos
goto-definition = Ir a definición
terminal = Terminal
command-palette = Paleta de comandos
minimap = Minimapa
cycle-theme = Cambiar tema
editing = Edición
about = Acerca de
refresh = Refrescar
reconnect = Reconectar

# === nada (editor de archivos) ===
nada-tagline = editor soberano sobre Llimphi
nada-settings-appearance = Apariencia
nada-settings-theme = Tema
nada-settings-editor = Editor
nada-settings-fmt-on-save = Formatear al guardar
nada-settings-demo-diag = Diagnósticos de demostración
nada-settings-lsp = Servidor de lenguaje

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
# menú principal
wawa-menu-file = Archivo
wawa-menu-reload = Recargar imagen
wawa-menu-quit = Salir
wawa-menu-view = Ver
wawa-menu-fetch = Traer nodo por AoE
wawa-menu-theme = Cambiar tema
wawa-menu-help = Ayuda
wawa-menu-about = Acerca de
# menú contextual sobre el nodo seleccionado
wawa-ctx-select = Seleccionar
wawa-ctx-expand = Expandir
wawa-ctx-collapse = Contraer
wawa-ctx-fetch = Traer por AoE

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
minga-menu-file = Archivo
minga-menu-view = Ver
minga-menu-help = Ayuda
minga-menu-refresh = Refrescar
minga-menu-quit = Salir
minga-menu-theme = Cambiar tema
minga-menu-about = Acerca de
minga-menu-context-title = Repo

# === nakui-explorer (event log) ===
nakui-explorer-header = Log: { $path }  ·  { $entries } entries ({ $seeds } seeds, { $morphisms } morphisms)  ·  reload { $ms } ms
nakui-explorer-breakdown = breakdown: { $parts }

# === supay (doom) ===
supay-mode-real = ENGINE REAL
supay-mode-stub = STUB
supay-view-fb = view=FB (F3→3D)
supay-view-3d = view=3D (F3→FB)
supay-view-wgpu = view=wgpu 2.5D (F3)
supay-header = { $title }  ·  tick { $tick }  ·  { $mode }  ·  { $view }  ·  { $scene }
supay-stub-title = No se pudo bajar el motor (doomgeneric) automáticamente
supay-stub-step-1 = Conectate a internet
supay-stub-step-1-cmd =     el motor C se clona solo durante la compilación
supay-stub-step-2 = Reconstruí supay
supay-stub-step-2-cmd =     cargo run -p supay-doom-llimphi --release
supay-stub-step-3 = El WAD ya se descarga con un botón al abrir
supay-stub-step-3-cmd =     (sin curl ni terminal — sólo apretar Descargar)
supay-stub-footer = doomgeneric (C) avanza a 35 Hz; el framebuffer 320×200 ARGB se pinta en aspect-fit.
supay-controls-hint = WASD · Ctrl disp · Space usa · Tab map · F3 vista · F4 mira · F5 viñeta · F6 HUD · F7 sombras · F8 fogonazo · F9 oclusión · F10 luz-mobj · F11 rim-arma · F12 salir
supay-stub-controls-hint = F3 alterna FB/3D  ·  F12 cierra la ventana
# Pantalla de adquisición del WAD (descarga in-app)
supay-wad-title = Falta el WAD de Doom — descargalo para jugar
supay-wad-dir-label = Directorio destino
supay-wad-download = Descargar WAD
supay-wad-downloading = Descargando el WAD shareware…
supay-wad-hint = Elegí un directorio y tocá Descargar (Enter). Bajamos el shareware ahí y arrancamos solos.
supay-wad-error = Falló la descarga
supay-wad-not-here = No hay ningún WAD en ese directorio
supay-wad-play-existing = Ya lo tengo — Jugar
supay-wad-legal = DOOM1.WAD shareware (episodio 1, ~4 MB, distribuible sin costo). Se guarda como doom1.wad.

# === shuma-shell ===
shuma-label-launcher = Launcher
shuma-label-command = Command
shuma-label-shell = Shell
shuma-label-matilda = Matilda
shuma-label-canvas = Lienzo
shuma-label-monitors = Monitores
shuma-empty-main-incompat = Módulo Main no compatible
shuma-empty-no-tabs = Sin tabs configuradas.
shuma-empty-no-tabs-compat = Este módulo no puede ser tab.
shuma-empty-no-data-linux = sin datos (¿no es Linux?)
shuma-empty-no-data = sin datos
shuma-stat-samples = muestras: { $have } / { $total }

# === nahual (visores) ===
nahual-image-unsupported = formato no soportado (sólo PNG/JPEG en esta build)
nahual-image-toobig = (archivo muy grande: { $bytes } bytes — sin preview)
nahual-image-error = (error: { $err })
nahual-image-select = (seleccioná una imagen)
nahual-image-empty-title = Sin imagen
nahual-image-empty-body = Seleccioná una imagen para previsualizarla.
nahual-audio-select = (seleccioná un audio)
nahual-audio-error = (error: { $err })
nahual-card-select = (seleccioná una card)
nahual-card-invalid = (card inválida: { $err })
nahual-fe-lines = Líneas
nahual-fe-no-entries = No hay entradas en { $path }
nahual-fe-empty = Carpeta vacía
nahual-fe-caption = { $n } entradas · ↑↓ navega · Enter entra · ⌫ sube
nahual-fe-more = … y { $n } más (rueda o ↓ para ver más)
nahual-archive-select = (seleccioná un ZIP/tar/tar.gz)
nahual-archive-error = (no se pudo abrir: { $err })

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

# === tawasuyu-edit (editor de código) ===
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
wawa-panel-menu-file = Archivo
wawa-panel-menu-view = Ver
wawa-panel-menu-help = Ayuda
wawa-panel-menu-quit = Salir
wawa-panel-status-hint = ↑↓ navegar  ·  Enter activar  ·  Ctrl+S guardar  ·  Esc salir
wawa-panel-about-name = Sistema
wawa-panel-about-version = Versión
wawa-panel-about-kernel = Núcleo
wawa-panel-about-toolkit = Toolkit
wawa-panel-about-blurb = wawa es el sistema operativo de la suite tawasuyu. El kernel arje y las apps llimphi sobre un userland mínimo.
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


# === ayni-llimphi ===
ayni-menu-admitir = Admitir seleccionado
ayni-menu-atestar = Atestar seleccionado
ayni-menu-expulsar = Expulsar seleccionado
ayni-menu-enviar-msg = Enviar mensaje
ayni-menu-adjuntar = Adjuntar archivo…
ayni-menu-acuse = Acuse de recibo
ayni-menu-cifrado = Cifrado E2EE
ayni-menu-recibos = Recibos de lectura
ayni-menu-comandos-barra = Comandos de la barra /
ayni-label-gente-miembros = GENTE — miembros
ayni-label-otros-vistos = otros vistos
ayni-label-acciones = acciones
ayni-label-elige-alguien = elegí a alguien arriba
ayni-btn-admitir = admitir
ayni-btn-atestar = atestar
ayni-btn-expulsar = expulsar
ayni-btn-acuse = acuse
ayni-label-confianza = confianza (saltos)
ayni-label-sin-atestaciones = — sin atestaciones —
ayni-label-sin-mensajes = — sin mensajes. Escribí abajo (o /ayuda para comandos). —
ayni-compose-placeholder = escribí un mensaje, o /adjuntar <ruta>, /atestar <hex> …
ayni-btn-enviar = enviar

# === chaka-app-llimphi ===
chaka-menu-run = Ejecutar
chaka-menu-run-pipeline = Correr pipeline
chaka-tab-output = Salida
chaka-tab-rust = Rust generado
chaka-tab-diag = Diagnósticos
chaka-btn-run = Correr
chaka-corpus-empty = corpus vacío
chaka-corpus-header = CORPUS
chaka-editor-placeholder = seleccioná un programa del corpus
chaka-no-file = sin archivo
chaka-banner-open-corpus = abrí un programa del corpus a la izquierda
chaka-banner-step-limit = shadow ⚠ se agotó el tope de pasos (¿bucle sin fin?)
chaka-banner-pipeline-error = el pipeline falló — ver tab «Diag» para detalles
chaka-status-no-open-file = no hay archivo abierto para guardar
chaka-about-text = chaka · transpilador COBOL → Rust · pipeline lex→parse→ir→codegen→shadow

# === chasqui-explorer-llimphi ===
chasqui-explorer-ctx-detail = Ver detalle
chasqui-explorer-monad-label = Mónada
chasqui-explorer-monad-stats = { $count } files · ent { $entropy } · { $lens }

# === media-app ===
media-settings-tab-audio = Audio
media-settings-tab-video = Video
media-settings-tab-playback = Reproducción
media-settings-tab-bars = Barras
media-settings-tab-controls = Controles
media-audio-volume = Volumen
media-audio-eq = Ecualizador
media-audio-normalization = Normalización
media-audio-lufs-target = Objetivo LUFS
media-audio-downmix = Downmix estéreo
media-video-color = Color
media-video-enable = Activar
media-video-brightness = Brillo
media-video-contrast = Contraste
media-video-gamma = Gamma
media-video-saturation = Saturación
media-video-hue = Matiz
media-video-orientation = Orientación
media-video-rotation = Rotación
media-video-rotate-cw = rotar 90°
media-video-flip-h = Espejo H
media-video-flip-v = Espejo V
media-action-reset = reset
media-action-cycle = ciclar
media-playback-playlist = Playlist
media-playback-resume = Reanudar al abrir
media-playback-repeat = Repetición
media-playback-shuffle = Aleatorio
media-playback-subtitles = Subtítulos
media-playback-autoload-sidecar = Auto-cargar sidecar
media-playback-sub-delay = Desfase (ms)
media-playback-font-size = Tamaño de letra
media-playback-behavior = Comportamiento
media-playback-crossfade = Crossfade (s)
media-controls-header = Controles (teclado)
media-controls-hint = Editá controles.ron y apretá F5 para reasignar teclas. El editor visual de atajos llega después.
media-bars-header = Barras de controles — clic en un item lo quita
media-bars-bar-label = Barra
media-bars-remove-bar = − quitar barra
media-bars-add-bar = + barra nueva
media-bars-on = encendida
media-bars-off = apagada
media-bars-autohide-on = autooculta
media-bars-autohide-off = siempre visible
media-dock-perfiles = Perfiles
media-prof-new = + nuevo perfil
media-prof-none = Sin perfiles. Creá uno.
media-prof-locked = con clave
media-prof-set-pass = poner clave
media-prof-clear-pass = quitar clave
media-prof-playlists = Playlists
media-prof-no-playlists = Sin playlists. Agregá una carpeta.
media-prof-add-dir = + desde carpeta…
media-prof-input-hint = Enter confirma · Esc cancela
media-prof-name-ph = nombre del perfil
media-prof-pass-ph = contraseña (vacío = sin clave)
media-prof-dir-ph = /ruta/al/directorio
media-prof-bad-name = nombre inválido o repetido
media-prof-bad-pass = contraseña incorrecta
media-prof-no-active = elegí un perfil activo primero
media-prof-no-media = el directorio no tiene medios
media-prof-tracks = pistas
media-prof-open = + abrir medio (o arrastrá un archivo)
media-prof-open-ph = /ruta/al/video-o-audio
media-bars-add-items-to = Agregar items a:
media-settings-footer = Se guarda en config.ron · Esc cierra · en Barras: clic en un item lo quita, ‹ › reordenan
media-playlist-header = Lista de reproducción — clic en una pista para saltar
media-playlist-empty = Sin lista de reproducción.
media-win-config-title = Configuración — media
media-win-playlist-title = Lista de reproducción — media
media-help-title = media · atajos
media-help-group-playback = Reproducción
media-help-toggle = Mostrar/ocultar esta ayuda
media-help-close = Cerrar la ayuda
media-help-reload = Recargar controles.ron en caliente
media-menu-capture-frame = Capturar fotograma
media-menu-record = Grabar / detener
media-menu-reload-controls = Recargar controles
media-menu-playback = Reproducción
media-menu-play-pause = Reproducir / pausar
media-menu-seek-back = Retroceder
media-menu-seek-fwd = Avanzar
media-menu-prev-track = Pista anterior
media-menu-next-track = Pista siguiente
media-menu-volume-up = Subir volumen
media-menu-volume-down = Bajar volumen
media-menu-playlist = Lista de reproducción
media-menu-visualizers = Visualizadores de audio
media-menu-shortcuts-help = Ayuda de atajos
media-ctx-stop-record = Detener grabación
media-ctx-record-audio = Grabar audio

# === mirada-app-llimphi ===
mirada-menu-open-window = Abrir ventana
mirada-menu-open-output = Abrir monitor
mirada-menu-close-focused = Cerrar enfocada
mirada-menu-window = Ventana
mirada-win-promote = Promover a maestra
mirada-win-float = Flotar / anclar
mirada-win-fullscreen = Pantalla completa
mirada-win-scratchpad = Enviar al scratchpad
mirada-win-label-fallback = ventana
mirada-layout-cycle = Ciclar layout
mirada-layout-master-stack = Maestro + pila
mirada-layout-monocle = Monóculo
mirada-layout-grid = Rejilla
mirada-layout-columns = Columnas
mirada-layout-rows = Filas
mirada-layout-centered = Maestro centrado
mirada-layout-spiral = Espiral
mirada-layout-shrink = Achicar maestra
mirada-layout-grow = Agrandar maestra
mirada-output-next = Siguiente monitor
mirada-view-overview = Vista espacial
mirada-status-body-connected = Cuerpo conectado
mirada-status-simulation = simulación — sin Cuerpo
mirada-status-keymap-reloaded = keymap recargado
mirada-status-keymap-invalid = keymap inválido
mirada-label-layout = layout
mirada-label-focus = foco
mirada-label-output = salida
mirada-label-workspace = escritorio
mirada-canvas-empty-hint = escritorio vacío — pulsa n para abrir una ventana
mirada-win-kind-fullscreen = · pantalla completa ·
mirada-win-kind-floating = · ventana flotante ·
mirada-win-kind-surface = · superficie del Cuerpo ·

# === mirada-greeter ===
mirada-greeter-menu-session = Sesión
mirada-greeter-menu-bg = Fondo
mirada-greeter-bg-chakana = Chakana (marca)
mirada-greeter-bg-matrix = Lluvia Matrix
mirada-greeter-bg-stars = Estrellas
mirada-greeter-bg-waves = Ondas
mirada-greeter-bg-fire = Fuego
mirada-greeter-bg-plasma = Plasma
mirada-greeter-bg-aurora = Aurora
mirada-greeter-bg-lightning = Rayos
mirada-greeter-bg-alleycat = Gato callejero
mirada-greeter-bg-off = Apagar fondo
mirada-greeter-session-submit = Iniciar sesión
mirada-greeter-session-goto-user = Ir a usuario
mirada-greeter-session-goto-pass = Ir a contraseña
mirada-greeter-label-desktop = Escritorio
mirada-greeter-btn-submit = Entrar
mirada-greeter-btn-submitting = Entrando…
mirada-greeter-hint-nav = ↑/↓: escritorio · Enter: entrar
mirada-greeter-hint-console = Ctrl+Alt+F1…F12: consola · Ctrl+Alt+⌫: salir
# Lock de pantalla (reusa la misma tarjeta que el greeter).
mirada-lock-subtitle = Pantalla bloqueada
mirada-lock-label-user = Sesión bloqueada
mirada-lock-btn = Desbloquear
mirada-lock-btn-busy = Desbloqueando…
mirada-lock-hint = Enter para desbloquear
mirada-lock-hint-switch = F2 para cambiar de usuario
mirada-lock-switch-cap = Cambiar a
mirada-lock-now-playing = Sonando ahora

# === nakui-explorer-llimphi ===
nakui-explorer-menu-refresh-log = Refrescar log
nakui-explorer-ctx-view-detail = Ver detalle
nakui-explorer-ctx-refresh-log = Refrescar log
nakui-explorer-ctx-entry-fallback = Entrada

# === nakui-sheet-llimphi ===
nakui-sheet-ctx-clear = Limpiar
nakui-sheet-fmt-number = Formato: Número
nakui-sheet-fmt-currency = Formato: Moneda $
nakui-sheet-fmt-percent = Formato: Porcentaje
nakui-sheet-fmt-general = Formato: General
nakui-sheet-freeze-here = Inmovilizar paneles aquí
nakui-sheet-unfreeze = Liberar paneles
nakui-sheet-pivot = Tabla dinámica…
nakui-sheet-menu-cell-cut = Cortar celda
nakui-sheet-menu-cell-copy = Copiar celda
nakui-sheet-menu-cell-paste = Pegar celda
nakui-sheet-menu-cell-clear = Limpiar celda
nakui-sheet-menu-bar-cut = Cortar texto
nakui-sheet-menu-bar-copy = Copiar texto
nakui-sheet-menu-bar-paste = Pegar texto
nakui-sheet-menu-bar-select-all = Seleccionar todo (texto)
nakui-sheet-menu-import-csv = Importar CSV
nakui-sheet-menu-export-csv = Exportar CSV
nakui-sheet-menu-about = Acerca de Nakui Sheet
nakui-sheet-formula-placeholder = ingresa fórmula o valor
nakui-sheet-pivot-title = Tabla dinámica
nakui-sheet-pivot-close = ✕ Esc
nakui-sheet-pivot-group-by = Agrupar por «
nakui-sheet-pivot-over = sobre
nakui-sheet-pivot-with-header = c/encab.
nakui-sheet-pivot-no-header = s/encab.
nakui-sheet-pivot-more-groups = grupos
nakui-sheet-pivot-total = TOTAL
nakui-sheet-pivot-groups = grupos
nakui-sheet-pivot-rows = filas
nakui-sheet-pivot-hint = A función · G grupo · V valor · H encabezado · Esc cerrar

# === paloma-llimphi ===
paloma-status-init = paloma · sin sincronizar
paloma-status-search-semantic = búsqueda por significado — escribí y presioná Enter
paloma-status-search-semantic-running = buscando por significado…
paloma-status-search-semantic-done = { $n } resultado(s) por significado
paloma-status-search-semantic-fallback = sin daemon de embeddings: usando búsqueda exacta
paloma-search-semantic-running = buscando por significado…
paloma-search-semantic-hint = escribí y presioná Enter para buscar por significado
paloma-btn-summarize = Resumir
paloma-btn-summarizing = Resumiendo…
paloma-btn-ai-draft = Borrador IA
paloma-btn-drafting = Redactando…
paloma-llm-summary-title = ✨ Resumen del hilo
paloma-llm-summarizing = resumiendo el hilo con IA…
paloma-status-llm-summarizing = resumiendo el hilo con IA…
paloma-status-llm-summary-done = resumen listo
paloma-status-llm-drafting = redactando un borrador con IA…
paloma-status-llm-draft-done = borrador listo · revisalo antes de enviar
paloma-status-view-rich = HTML enriquecido vía puriy: pendiente (texto despojado por ahora)
paloma-status-no-recipient = no se puede enviar: falta un destinatario válido
paloma-status-sent = enviado
paloma-status-sent-signed = enviado · firmado (Ed25519)
paloma-status-sent-unsigned-nokey = enviado SIN firmar · no hay identidad configurada
paloma-status-rail-sent = enviado por el rail P2P (sin SMTP)
paloma-status-rail-received = recibido por el rail P2P · buzón Suyu
paloma-btn-my-rail = Mi dirección Suyu
paloma-btn-add-contact = Contacto
paloma-trust-known = { $name }
paloma-trust-unknown = remitente no guardado
paloma-trust-vouched = avalado por { $name }
paloma-btn-vouch = Avalar
paloma-status-vouched = avalaste a { $name } · tu firma respalda su identidad
paloma-status-vouch-not-rail = sólo se puede avalar identidades del rail (firmadas)
paloma-status-contact-added = «{ $name }» agregado a contactos
paloma-status-contact-updated = «{ $name }» actualizado en contactos
paloma-status-llm-translating = traduciendo el lienzo con IA…
paloma-status-llm-lienzo-done = lienzo {lang} listo · viajará con el mensaje
paloma-btn-lienzo = Lienzo
paloma-read-original = Original
paloma-placeholder-search = Buscar… ( / )
paloma-btn-compose = ✎ Redactar
paloma-nav-calendar = Calendario
paloma-nav-contacts = Contactos
paloma-nav-soon = pronto
paloma-empty-threads = Bandeja vacía
paloma-empty-search = sin coincidencias
paloma-search-exact = Exacta
paloma-search-semantic = Semántica
paloma-no-subject = (sin asunto)
paloma-placeholder-read = Elegí un hilo para leerlo
paloma-btn-reply = Responder
paloma-btn-forward = Reenviar
paloma-btn-star = Destacar
paloma-btn-starred = Destacado
paloma-btn-mark-unread = Marcar no leído
paloma-btn-mark-read = Marcar leído
paloma-btn-view-rich = Ver HTML enriquecido
paloma-msg-to-label = para
paloma-sig-verified = firmado
paloma-sig-invalid = firma inválida
paloma-compose-new = Mensaje nuevo
paloma-compose-reply-title = Responder
paloma-compose-placeholder-to = Para: nombre <correo@dominio>
paloma-compose-placeholder-cc = Cc: (opcional)
paloma-compose-placeholder-subject = Asunto
paloma-compose-placeholder-body = Escribí tu mensaje…
paloma-compose-sign = Firmar (Ed25519)
paloma-compose-send = Enviar

# === pluma-notebook-llimphi ===
pluma-notebook-fit-all = Ajustar todo
pluma-notebook-center = Centrar
pluma-notebook-zoom-reset = Zoom 100%

# === raymi-llimphi ===
raymi-tab-calendar = Calendario
raymi-tab-contacts = Contactos
raymi-view-month = Mes
raymi-view-week = Semana
raymi-view-day = Día
raymi-btn-new-event = ＋ Evento
raymi-btn-today = Hoy
raymi-btn-new-contact = ＋ Contacto
raymi-no-events = sin eventos
raymi-all-day = todo el día
raymi-no-contacts = sin contactos
raymi-search-contact-placeholder = 🔍 Buscar contacto…
raymi-select-contact-hint = Elegí un contacto
raymi-title-edit-event = Editar evento
raymi-title-new-event = Nuevo evento
raymi-title-edit-contact = Editar contacto
raymi-title-new-contact = Nuevo contacto
raymi-change-cycle = cambiar
raymi-field-summary = Asunto
raymi-field-all-day = Día completo
raymi-field-apply-to = Aplicar a
raymi-field-calendar = Calendario
raymi-field-date = Fecha
raymi-field-start = Inicio
raymi-field-end = Fin
raymi-field-location = Lugar
raymi-ph-location = Lugar (opcional)
raymi-field-description = Descripción
raymi-ph-description = Notas (opcional)
raymi-field-attendees = Invitados
raymi-ph-invitee = Nombre <correo> · Enter
raymi-field-repeat = Repetir
raymi-field-every = Cada
raymi-field-days = Días
raymi-field-ends = Termina
raymi-field-name = Nombre
raymi-field-emails = Correos
raymi-field-phones = Teléfonos
raymi-field-org = Organización
raymi-field-note = Nota
raymi-ph-full-name = Nombre y apellido
raymi-ph-emails = correo@dominio, otro@…
raymi-ph-phones = +58 412…, …
raymi-ph-org = Empresa (opcional)
raymi-ph-note = Nota (opcional)
raymi-scope-series = Toda la serie
raymi-scope-this-only = Esta instancia
raymi-scope-this-and-future = Esta y siguientes
raymi-repeat-none = No se repite
raymi-repeat-daily = Diariamente
raymi-repeat-weekly = Semanalmente
raymi-repeat-monthly = Mensualmente
raymi-repeat-yearly = Anualmente
raymi-unit-days = día(s)
raymi-unit-weeks = semana(s)
raymi-unit-months = mes(es)
raymi-unit-years = año(s)
raymi-end-never = Sin fin
raymi-end-count = Tras N veces
raymi-end-until = Hasta fecha
raymi-status-no-calendars = no hay calendarios donde crear un evento
raymi-status-no-books = no hay libretas donde crear un contacto
raymi-status-invalid-datetime = fecha u hora inválida (usá AAAA-MM-DD y HH:MM)
raymi-status-contact-needs-name = el contacto necesita un nombre

# === shuma-shell-llimphi ===
shuma-shell-clear-input = Limpiar entrada
shuma-shell-clear-screen = Limpiar pantalla
shuma-shell-cancel-cmd = Cancelar comando
shuma-shell-about = Acerca de shuma
shuma-layouts = Disposiciones…

# === supay-app-llimphi ===
supay-hud-health = VIDA
supay-hud-ammo = MUNICION
supay-hud-target = OBJETIVO
supay-action-fire = Disparar
supay-action-reset = Reiniciar partida
supay-menu-play = Jugar
supay-status-game-over = fin de partida
supay-status-victory = victoria
supay-status-dead = MUERTO
supay-hint-space-restart = SPACE para reiniciar

# === wawa-panel-llimphi ===
wawa-panel-status-config-updated = ↻ config actualizada desde el bus
wawa-panel-ctx-refresh-monitor = Refrescar monitor
wawa-panel-autosave-ok = ↻ aplicado
