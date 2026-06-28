// registrar_cliente — alta de un Cliente con verificación del DÍGITO
// VERIFICADOR del documento fiscal (aritmética módulo 11/10 que el regex de
// Nickel no puede hacer).
//
// Reparto de responsabilidades:
//   · FORMA del documento (cédula/RIF/CUIT/RUT…) → contrato Nickel
//     (modules/_fiscal/documentos.ncl), verificado por el kernel en el
//     post-check de toda entidad creada (executor.rs paso 8).
//   · DÍGITO VERIFICADOR → el módulo Rhai compartido "fiscal"
//     (modules/_fiscal/dv.rhai), registrado por el RhaiExecutor. Si no
//     cuadra, `throw` y el alta no ocurre.
import "fiscal" as f;

let pais = input.params.pais;
let doc = input.params.documento;

if !f::dv_ok(pais, doc) {
    throw "documento inválido para " + pais + ": el dígito verificador no coincide (" + doc + ")";
}

[
    #{ op: "create", entity: "Cliente", id: input.params.id,
       data: #{
           id: input.params.id,
           nombre: input.params.nombre,
           email: input.params.email,
           empresa: input.params.empresa,
           pais: pais,
           documento: doc,
       } },
]
