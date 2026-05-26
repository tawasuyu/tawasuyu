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
