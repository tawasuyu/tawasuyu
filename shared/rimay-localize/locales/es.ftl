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
cosmos-header = cosmos · { $title } (mock · Asc { $asc }° MC { $mc }°)
cosmos-demo-title = Carta sintética (demo)
cosmos-demo-subtitle = sin cómputo real — sólo geometría

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
