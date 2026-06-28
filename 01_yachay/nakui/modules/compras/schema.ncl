# Esquema del módulo Compras (Nickel contracts) — facturas de compra
# (vendor bills) y pagos a proveedor que ASIENTAN en el libro de
# `accounting`. Es el espejo de `facturacion` del lado de cuentas por
# pagar.
#
# Cross-module: el `nsmc.json` cruza el schema de accounting (Cuenta) y
# agrega Compra + Pago. `registrar_compra` liga tres cuentas (la de
# gasto/inventario, IVA crédito fiscal y Proveedores) y las mueve de forma
# balanceada; `pagar` mueve el total de Proveedores a Banco. El kernel
# exige Σ Δ Cuenta.saldo = 0 por moneda en ambos.
#
# Importes en unidades enteras de moneda.
let NoNegativo = std.contract.from_predicate (fun n => std.is_number n && n >= 0) in
# Pack fiscal compartido: valida el formato del documento del proveedor (RIF,
# cédula, CUIT…) según `pais`. Opt-in — una compra sin `documento` sigue
# siendo válida; con `documento` exige `pais` y respeta el formato del país.
# El dígito verificador lo chequea el morfismo (módulo Rhai "fiscal").
let fiscal = import "../_fiscal/documentos.ncl" in
{
  # Una factura de compra registrada. La crea `registrar_compra`.
  # Cabecera con los importes ya calculados. Opcionalmente lleva
  # {pais, documento} del proveedor para identificación fiscal legal.
  Compra = std.contract.Sequence [
    fiscal.ConDocumentoFiscal,
    {
      proveedor | String,
      neto | NoNegativo,
      impuesto | NoNegativo,
      total | NoNegativo,
      moneda | String,
      ..
    },
  ],

  # Constancia append-only del pago de una compra. La crea `pagar`: mueve
  # el total de Proveedores a Banco y marca la compra como pagada.
  Pago = {
    compra_id | String,
    monto | NoNegativo,
    moneda | String,
    ..
  },
}
