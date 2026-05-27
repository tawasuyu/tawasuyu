# `ui_module_basic.ncl` — template canónico para Cards de tipo
# UiModule (descriptores de módulos para metainterfaz yahweh).
#
# Use case típico: declarar un módulo nuevo sobrescribiendo `id`,
# `label`, y aportando los `entities`/`menu`/`views` propios:
#
#     let base = import "ui_module_basic.ncl" in
#     base & {
#       id = "customers",
#       label = "Clientes",
#       entities = [
#         { name = "Customer", label = "Cliente", fields = [...] },
#       ],
#       menu = [{ label = "Listar", view = "list" }],
#       views = { list = { kind = "list", ... } },
#     }
#
# El brazo lo dispatcha al `UiModuleJsonReader` por la presencia
# simultánea de `entities` Y `views` Y `menu`.

{
  # Identidad: override siempre.
  id | String | default = "TEMPLATE_ID",
  label | String | default = "TEMPLATE_LABEL",

  # Subtítulo opcional (tooltip en el sidebar). null por default.
  description | default = null,

  # Las 3 listas/maps son el **payload** real del módulo. El
  # template las deja vacías para que el usuario las defina sin
  # heredar nada útil-pero-equivocado de un default.
  entities | default = [],
  menu | default = [],
  views | default = {},
}
