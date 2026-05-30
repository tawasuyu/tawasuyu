pub(crate) const PAYMENT_BOOTSTRAP: &str = r#"
// Fase 7.113 — Payment Request API (`PaymentRequest`). Los checkouts la usan para
// invocar la hoja de pago nativa del navegador en vez de un formulario propio.
// `new PaymentRequest(methodData, details, options)` valida y `.show()` abre la
// hoja → Promise<PaymentResponse>. El motor no tiene hoja de pago: `show()`
// publica una mutación `kind: 'payment-request'` (value `<id>:<json>`) y devuelve
// una Promise pendiente que el chrome resuelve con `__puriy_payment_resolve(id, data)`
// (construye un PaymentResponse) o rechaza con `__puriy_payment_reject(id, name, msg)`
// — patrón pending-id como credentials 7.110. `canMakePayment()` → host-decided
// (`__puriy_set_payment_can_make(bool)`, default true). `PaymentResponse.complete()`
// publica `kind: 'payment-complete'`. PaymentRequest hereda de EventTarget (Fase
// 7.76) para los eventos shipping*change. Wiring de la hoja real pendiente.
(function() {
    if (globalThis.PaymentRequest != null) return;

    globalThis.__puriy_payment_pending = globalThis.__puriy_payment_pending || {};
    globalThis.__puriy_payment_next_id = globalThis.__puriy_payment_next_id || 1;

    function PaymentResponse(requestId, data) {
        data = data || {};
        this.requestId = String(requestId);
        this.methodName = data.methodName != null ? String(data.methodName) : '';
        this.details = data.details || {};
        this.payerName = data.payerName != null ? String(data.payerName) : null;
        this.payerEmail = data.payerEmail != null ? String(data.payerEmail) : null;
        this.payerPhone = data.payerPhone != null ? String(data.payerPhone) : null;
        this.shippingAddress = data.shippingAddress || null;
        this.shippingOption = data.shippingOption || null;
    }
    PaymentResponse.prototype.complete = function(result) {
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'payment-complete', value: String(result != null ? result : 'unknown')
        });
        return Promise.resolve(undefined);
    };
    PaymentResponse.prototype.retry = function() { return Promise.resolve(undefined); };
    globalThis.PaymentResponse = PaymentResponse;

    function PaymentRequest(methodData, details, options) {
        if (!Array.isArray(methodData) || methodData.length === 0) {
            throw new TypeError('PaymentRequest: methodData requerido');
        }
        if (details == null || details.total == null) {
            throw new TypeError('PaymentRequest: details.total requerido');
        }
        globalThis.EventTarget.call(this);
        this._methodData = methodData;
        this._details = details;
        this._options = options || {};
        this.id = (details.id != null) ? String(details.id) : ('pay-' + globalThis.__puriy_payment_next_id);
        this._shown = false;
        this._pendId = null;
        this.onshippingaddresschange = null;
        this.onshippingoptionchange = null;
        this.onpaymentmethodchange = null;
    }
    PaymentRequest.prototype = Object.create(globalThis.EventTarget.prototype);
    PaymentRequest.prototype.constructor = PaymentRequest;
    PaymentRequest.prototype.show = function() {
        var self = this;
        return new Promise(function(resolve, reject) {
            if (self._shown) {
                reject(new globalThis.DOMException('PaymentRequest ya mostrado', 'InvalidStateError'));
                return;
            }
            self._shown = true;
            var id = globalThis.__puriy_payment_next_id++;
            self._pendId = id;
            globalThis.__puriy_payment_pending[id] = { resolve: resolve, reject: reject, request: self };
            var summary = { id: self.id, total: self._details.total };
            globalThis.__puriy_dirty.push({
                id: '__window__', kind: 'payment-request', value: id + ':' + JSON.stringify(summary)
            });
        });
    };
    PaymentRequest.prototype.abort = function() {
        var id = this._pendId;
        var p = (id != null) ? globalThis.__puriy_payment_pending[id] : null;
        if (p) {
            delete globalThis.__puriy_payment_pending[id];
            p.reject(new globalThis.DOMException('Pago abortado', 'AbortError'));
        }
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'payment-abort', value: String(this.id) });
        return Promise.resolve(undefined);
    };
    PaymentRequest.prototype.canMakePayment = function() {
        return Promise.resolve(globalThis.__puriy_payment_can_make !== false);
    };
    PaymentRequest.prototype.hasEnrolledInstrument = function() {
        return Promise.resolve(globalThis.__puriy_payment_can_make !== false);
    };
    globalThis.PaymentRequest = PaymentRequest;

    globalThis.__puriy_payment_resolve = function(id, data) {
        var p = globalThis.__puriy_payment_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_payment_pending[id];
        p.resolve(new PaymentResponse(p.request.id, data));
        return true;
    };
    globalThis.__puriy_payment_reject = function(id, name, msg) {
        var p = globalThis.__puriy_payment_pending[id];
        if (!p) return false;
        delete globalThis.__puriy_payment_pending[id];
        p.reject(new globalThis.DOMException(msg || 'Pago rechazado', name || 'AbortError'));
        return true;
    };
    globalThis.__puriy_set_payment_can_make = function(can) {
        globalThis.__puriy_payment_can_make = !!can;
        return true;
    };
})();
"#;
