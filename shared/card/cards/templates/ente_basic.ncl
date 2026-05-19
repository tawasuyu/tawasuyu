# `ente_basic.ncl` — template canónico para Cards de tipo Ente.
#
# Use case típico: declarar una entity runtime mínima (Virtual
# payload, OneShot supervision) sobrescribiendo sólo `id` y `label`:
#
#     let base = import "ente_basic.ncl" in
#     base & {
#       id = "01ARZ3NDEKTSV4RRFFQ69G5FAV",
#       label = "mi-ente",
#     }
#
# El brazo `brahman-cards::load_card` lo dispatcha al
# `EnteJsonReader` porque el shape resultante tiene `payload` Y
# `supervision` (los campos detect-key del reader Ente).
#
# **Convención obligatoria**: cada field que el usuario va a
# sobrescribir está marcada `| default`. Sin eso Nickel rebota el
# merge de strings/numbers no-iguales con misma prioridad.

{
  schema_version | Number | default = 1,

  # Identidad: el usuario casi siempre las sobrescribe.
  id | String | default = "TEMPLATE_ID",
  label | String | default = "TEMPLATE_LABEL",

  # Runtime defaults razonables: nodo lógico sin proceso, sin
  # restart. Override si querés un ente con payload Wasm/Native.
  payload | default = "Virtual",
  supervision | default = "OneShot",
}
