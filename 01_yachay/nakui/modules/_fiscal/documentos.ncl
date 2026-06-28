# Pack fiscal compartido de nakui — validación de documentos de identidad
# tributaria por país (cédula, RIF, CUIT, RUC, RUT, NIT, RFC…).
#
# POR QUÉ ESTO EXISTE: el formato del documento fiscal cambia por país, pero
# el ERP no debe forkearse por país. Acá vive UNA fuente de verdad: un mapa
# `país → patrón`, más un contrato de record reusable que cualquier módulo
# (crm, facturacion, compras) importa para validar su entidad-tercero.
#
# CÓMO LO CONSUME UN MÓDULO (ver modules/crm/schema.ncl como referencia):
#
#   let fiscal = import "../_fiscal/documentos.ncl" in
#   {
#     Cliente = std.contract.Sequence [
#       fiscal.ConDocumentoFiscal,                 # valida pais+documento
#       { nombre | String, .., },                  # forma propia de la entidad
#     ],
#   }
#
# CONTRATO DE LA VALIDACIÓN (deliberadamente tolerante, para no bloquear el
# negocio por una regla que todavía no cargamos):
#   · Sin campo `documento` → pasa. Un tercero sin datos fiscales es válido
#     (retrocompatibilidad: los seeds viejos de crm no traen documento).
#   · Con `documento` pero sin `pais` → REBOTA. Si das un documento tenés que
#     decir de qué país es; si no, no se puede validar nada.
#   · Con `documento` y `pais` que SÍ tiene patrón → valida el formato.
#   · Con `documento` y `pais` SIN patrón declarado → pasa. No conocemos la
#     regla todavía; preferimos no trabar la carga a inventar un rechazo.
#
# ALCANCE: validación de FORMA (shape) vía regex. El dígito verificador
# (módulo 11 de RUT/CUIT/RIF, etc.) es una capa aparte — iría en un morfismo
# Rhai que normaliza/verifica al crear el tercero. Acá NO se calcula.
#
# AGREGAR UN PAÍS: sumá una línea a `patrones` con su código ISO-3166 alpha-2
# y el regex (motor `regex` de Rust). Mantené los patrones SIN backslash:
# usá `[0-9]` en vez de `\d` y `[.]` para un punto literal — Nickel rechaza
# escapes desconocidos como `\d` dentro de strings normales.
#
# LÍMITE CONOCIDO (determinismo): el bundle-hash del módulo (executor.rs,
# read_schema_files_concat) sólo cubre los `schema.ncl` declarados en el
# nsmc.json, NO este archivo importado transitivamente. Editar un patrón acá
# NO mueve el hash del módulo que lo importa. Para validación corriente es
# correcto; sólo afecta la detección de evolución-de-schema sobre snapshots
# viejos. Si algún día importa, declarar este archivo en `schemas` también.

let patrones = {
  # Venezuela — cédula (V/E + 6-8 díg) y RIF (V/E/J/P/G + 8-9 díg + verificador).
  VE = "^[VEJPGvejpg]-?[0-9]{6,9}(-[0-9])?$",
  # Bolivia — CI / NIT (dígitos, con sufijo opcional de extensión o verificador).
  BO = "^[0-9]{5,12}(-[0-9A-Za-z]{1,3})?$",
  # Perú — DNI (8 díg) o RUC (11 díg).
  PE = "^([0-9]{8}|[0-9]{11})$",
  # Ecuador — cédula (10 díg) o RUC (13 díg).
  EC = "^([0-9]{10}|[0-9]{13})$",
  # Colombia — cédula / NIT (6-10 díg, verificador opcional).
  CO = "^[0-9]{6,10}-?[0-9]?$",
  # Chile — RUT con puntos/guion opcionales; verificador 0-9 o K.
  CL = "^[0-9]{1,2}[.]?[0-9]{3}[.]?[0-9]{3}-?[0-9kK]$",
  # Argentina — CUIT / CUIL (2 + 8 + 1 verificador).
  AR = "^[0-9]{2}-?[0-9]{8}-?[0-9]$",
  # México — RFC persona física (13) o moral (12): 3-4 letras + AAMMDD + homoclave.
  MX = "^[A-Za-zÑñ&]{3,4}[0-9]{6}[A-Za-z0-9]{3}$",
} in

# ¿`valor` es un documento bien formado para `pais`? País sin patrón → true
# (no conocemos la regla, no bloqueamos). Currificada: Nickel no acepta
# `fun a b =>` multi-argumento en esta posición.
let valida = fun pais => fun valor =>
  if std.record.has_field pais patrones then
    std.string.is_match (std.record.get pais patrones) valor
  else
    true
in

{
  # Mapa país→regex, expuesto por si un morfismo Rhai quiere el patrón crudo.
  patrones = patrones,

  # Predicado reusable país×valor → Bool.
  es_documento_valido = valida,

  # Contrato de record para entidades-tercero (Cliente, Proveedor…). Tolerante
  # con terceros sin datos fiscales; estricto cuando hay documento.
  ConDocumentoFiscal = std.contract.from_predicate (fun c =>
    if std.record.has_field "documento" c then
      std.record.has_field "pais" c && valida c.pais c.documento
    else
      true),
}
