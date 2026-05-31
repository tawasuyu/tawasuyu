pub(crate) const DOMEXCEPTION_BOOTSTRAP: &str = r#"
// Fase 7.72 — DOMException. Muchas APIs web (fetch/abort/FileReader/IndexedDB)
// rechazan/tiran con un DOMException de `name` bien definido (AbortError,
// TimeoutError, NotFoundError…), y las librerías hacen `e instanceof
// DOMException` o miran `e.name`. Acá una implementación fiel: hereda de Error
// (para `instanceof Error`), expone `name`/`message`/`code` y los 25 códigos
// legacy como constantes estáticas + en el prototype.
//
// Nota: por ahora NO retrofiteamos los sitios que tiran (fetch/abort siguen con
// Error) para no romper tests que miran `e.message`; DOMException queda
// disponible para que el user code y futuras fases lo adopten.
globalThis.__puriy_dom_exception_codes = {
    IndexSizeError: 1, HierarchyRequestError: 3, WrongDocumentError: 4,
    InvalidCharacterError: 5, NoModificationAllowedError: 7, NotFoundError: 8,
    NotSupportedError: 9, InUseAttributeError: 10, InvalidStateError: 11,
    SyntaxError: 12, InvalidModificationError: 13, NamespaceError: 14,
    InvalidAccessError: 15, SecurityError: 18, NetworkError: 19, AbortError: 20,
    URLMismatchError: 21, QuotaExceededError: 22, TimeoutError: 23,
    InvalidNodeTypeError: 24, DataCloneError: 25
};
globalThis.DOMException = function(message, name) {
    this.message = (message !== undefined) ? String(message) : '';
    this.name = (name !== undefined) ? String(name) : 'Error';
    var codes = globalThis.__puriy_dom_exception_codes;
    this.code = Object.prototype.hasOwnProperty.call(codes, this.name) ? codes[this.name] : 0;
};
globalThis.DOMException.prototype = Object.create(Error.prototype);
globalThis.DOMException.prototype.constructor = globalThis.DOMException;
globalThis.DOMException.prototype.toString = function() {
    return this.name + ': ' + this.message;
};
// Constantes legacy (en el constructor y en el prototype, como el spec).
globalThis.DOMException.INDEX_SIZE_ERR = 1;
globalThis.DOMException.HIERARCHY_REQUEST_ERR = 3;
globalThis.DOMException.WRONG_DOCUMENT_ERR = 4;
globalThis.DOMException.INVALID_CHARACTER_ERR = 5;
globalThis.DOMException.NO_MODIFICATION_ALLOWED_ERR = 7;
globalThis.DOMException.NOT_FOUND_ERR = 8;
globalThis.DOMException.NOT_SUPPORTED_ERR = 9;
globalThis.DOMException.INUSE_ATTRIBUTE_ERR = 10;
globalThis.DOMException.INVALID_STATE_ERR = 11;
globalThis.DOMException.SYNTAX_ERR = 12;
globalThis.DOMException.INVALID_MODIFICATION_ERR = 13;
globalThis.DOMException.NAMESPACE_ERR = 14;
globalThis.DOMException.INVALID_ACCESS_ERR = 15;
globalThis.DOMException.SECURITY_ERR = 18;
globalThis.DOMException.NETWORK_ERR = 19;
globalThis.DOMException.ABORT_ERR = 20;
globalThis.DOMException.URL_MISMATCH_ERR = 21;
globalThis.DOMException.QUOTA_EXCEEDED_ERR = 22;
globalThis.DOMException.TIMEOUT_ERR = 23;
globalThis.DOMException.INVALID_NODE_TYPE_ERR = 24;
globalThis.DOMException.DATA_CLONE_ERR = 25;
"#;
