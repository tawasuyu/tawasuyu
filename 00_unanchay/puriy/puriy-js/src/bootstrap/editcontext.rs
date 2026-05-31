pub(crate) const EDITCONTEXT_BOOTSTRAP: &str = r#"
// Fase 7.169 — EditContext API (`new EditContext()`).
// Desacopla la edición de texto del DOM: editores avanzados (code editors, canvas-based
// docs, terminales web) gestionan su propio buffer/selección y reciben los eventos de IME
// (composición CJK, autocorrección móvil) sin un <textarea> oculto. Es un EventTarget puro
// con `text`/`selectionStart`/`selectionEnd` mutables vía `updateText`/`updateSelection`, y
// eventos `textupdate` (cambios desde el IME), `textformatupdate`, `characterboundsupdate`,
// `compositionstart`/`compositionend`. El chrome enchufa el IME real llamando
// `__puriy_editcontext_text_update(id, {...})` sobre el contexto activo (PENDIENTE — sin
// IME nativo todavía). `element.editContext = ctx` asocia el contexto a un elemento (acá
// sólo se guarda; el cableado de foco real es del chrome).
(function() {
    if (globalThis.EditContext != null) return;
    if (typeof globalThis.EventTarget !== 'function') return;  // requiere Fase 7.76

    function EditContext(options) {
        globalThis.EventTarget.call(this);
        options = options || {};
        this.text = String(options.text != null ? options.text : '');
        this.selectionStart = (options.selectionStart != null) ? (options.selectionStart | 0) : 0;
        this.selectionEnd = (options.selectionEnd != null) ? (options.selectionEnd | 0) : this.selectionStart;
        this.characterBoundsRangeStart = 0;
        this._characterBounds = [];
        this._controlBounds = null;
        this._selectionBounds = null;
        this._attached = [];
        this.ontextupdate = null;
        this.ontextformatupdate = null;
        this.oncharacterboundsupdate = null;
        this.oncompositionstart = null;
        this.oncompositionend = null;
        var reg = globalThis.__puriy_editcontexts = globalThis.__puriy_editcontexts || {};
        this._id = (globalThis.__puriy_editcontext_next_id = (globalThis.__puriy_editcontext_next_id || 0) + 1);
        reg[this._id] = this;
    }
    EditContext.prototype = Object.create(globalThis.EventTarget.prototype);
    EditContext.prototype.constructor = EditContext;

    EditContext.prototype.updateText = function(rangeStart, rangeEnd, text) {
        rangeStart = rangeStart | 0; rangeEnd = rangeEnd | 0; text = String(text);
        if (rangeStart > rangeEnd) { var t = rangeStart; rangeStart = rangeEnd; rangeEnd = t; }
        this.text = this.text.slice(0, rangeStart) + text + this.text.slice(rangeEnd);
    };
    EditContext.prototype.updateSelection = function(start, end) {
        this.selectionStart = start | 0;
        this.selectionEnd = (end != null) ? (end | 0) : (start | 0);
    };
    EditContext.prototype.updateControlBounds = function(rect) { this._controlBounds = rect || null; };
    EditContext.prototype.updateSelectionBounds = function(rect) { this._selectionBounds = rect || null; };
    EditContext.prototype.updateCharacterBounds = function(rangeStart, bounds) {
        this.characterBoundsRangeStart = rangeStart | 0;
        this._characterBounds = Array.isArray(bounds) ? bounds.slice() : [];
    };
    EditContext.prototype.characterBounds = function() { return this._characterBounds.slice(); };
    EditContext.prototype.attachedElements = function() { return this._attached.slice(); };

    globalThis.EditContext = EditContext;

    // El chrome llama esto cuando el IME confirma/compone texto sobre el contexto activo.
    // detail: { updateRangeStart, updateRangeEnd, text, selectionStart, selectionEnd }.
    globalThis.__puriy_editcontext_text_update = function(id, detail) {
        var reg = globalThis.__puriy_editcontexts || {};
        var ctx = reg[id];
        if (!ctx) return false;
        detail = detail || {};
        var rs = (detail.updateRangeStart != null) ? (detail.updateRangeStart | 0) : ctx.selectionStart;
        var re = (detail.updateRangeEnd != null) ? (detail.updateRangeEnd | 0) : ctx.selectionEnd;
        var txt = String(detail.text != null ? detail.text : '');
        ctx.updateText(rs, re, txt);
        if (detail.selectionStart != null) {
            ctx.updateSelection(detail.selectionStart, detail.selectionEnd);
        }
        var ev = new globalThis.Event('textupdate', {});
        ev.updateRangeStart = rs;
        ev.updateRangeEnd = re;
        ev.text = txt;
        ev.selectionStart = ctx.selectionStart;
        ev.selectionEnd = ctx.selectionEnd;
        if (typeof ctx.ontextupdate === 'function') {
            try { ctx.ontextupdate.call(ctx, ev); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
        ctx.dispatchEvent(ev);
        return true;
    };
    void 0;
})();
"#;
