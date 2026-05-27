# `monad_basic.ncl` — template canónico para Cards de tipo Monad.
#
# Use case típico: declarar una agrupación semántica de archivos
# (Mónada de Nouser) con metadata mínima:
#
#     let base = import "monad_basic.ncl" in
#     base & {
#       id = "01ARZ3NDEKTSV4RRFFQ69G5FAW",
#       label = "fotos-2026",
#       members = ["01ARZ3FILE1", "01ARZ3FILE2"],
#       cardinality = 2,
#     }
#
# El brazo lo dispatcha al `MonadJsonReader` por la presencia
# simultánea de `members` Y `cardinality`.

{
  schema_version | Number | default = 1,

  # Identidad: override siempre.
  id | String | default = "TEMPLATE_ID",
  label | String | default = "TEMPLATE_LABEL",

  # Metadata semántica: defaults vacíos. El usuario typically
  # override `members` + `cardinality`, opcionalmente `summary`
  # / `keywords` / `dominant_lens`.
  summary | String | default = "",
  keywords | default = [],
  centroid | default = [],
  cardinality | Number | default = 0,
  entropy | Number | default = 0.0,
  # Lens variants serialize lowercase (serde rename_all): grid /
  # code / gallery / database / markdown / tree.
  dominant_lens | default = "grid",

  # Membership: vacío por default. El usuario llena con los IDs
  # de archivo cuando los conoce.
  members | default = [],
  pins | default = [],

  # Timestamps Unix ms — default 0 = "no timestamp registrado".
  # Override con el momento real cuando importa.
  created_at_ms | Number | default = 0,
  updated_at_ms | Number | default = 0,
}
