pub(crate) const VIEWTRANSITIONS_BOOTSTRAP: &str = r#"
// Fase 7.139 — View Transitions API (`document.startViewTransition`). Anima la transición
// entre dos estados del DOM: la app pasa un `updateCallback` que muta el DOM, y el navegador
// captura un "antes" y un "después" y los cross-fade. Modelo 100% JS-puro y funcional para el
// contrato que las apps observan (corre el callback y resuelve las promesas); la animación real
// por frame queda del lado del chrome (no interpolamos pseudo-elementos todavía). Cuelga de
// `document` (lo augmenta como el resto de los bootstraps; set_document lo reemplazaría — los
// tests llaman document.startViewTransition directo).
(function() {
    var doc = globalThis.document = globalThis.document || {};
    if (typeof doc.startViewTransition === 'function') return;

    function ViewTransition(updateCallback) {
        var self = this;
        this.types = (typeof Set === 'function') ? new Set() : [];
        var ucDoneRes, ucDoneRej, readyRes, readyRej, finRes, finRej;
        this.updateCallbackDone = new Promise(function(res, rej) { ucDoneRes = res; ucDoneRej = rej; });
        this.ready = new Promise(function(res, rej) { readyRes = res; readyRej = rej; });
        this.finished = new Promise(function(res, rej) { finRes = res; finRej = rej; });
        this._skipped = false;

        // Corre el callback (puede devolver una promesa). updateCallbackDone refleja su settle;
        // ready resuelve tras la actualización del DOM; finished resuelve cuando "termina" la
        // animación (acá, inmediatamente después de ready, porque no animamos por frame).
        var ran;
        try {
            ran = (typeof updateCallback === 'function') ? Promise.resolve(updateCallback()) : Promise.resolve();
        } catch (e) {
            ran = Promise.reject(e);
        }
        ran.then(
            function() {
                ucDoneRes();
                if (self._skipped) {
                    var ab = new globalThis.DOMException('transición saltada', 'AbortError');
                    readyRej(ab); self.ready.catch(function() {});
                } else {
                    readyRes();
                }
                finRes();
            },
            function(err) {
                ucDoneRej(err); self.updateCallbackDone.catch(function() {});
                readyRej(err); self.ready.catch(function() {});
                finRej(err); self.finished.catch(function() {});
            }
        );

        this.skipTransition = function() { self._skipped = true; };
    }

    doc.startViewTransition = function(updateCallback) {
        return new ViewTransition(updateCallback);
    };
    globalThis.ViewTransition = ViewTransition;
    void 0;
})();
"#;
