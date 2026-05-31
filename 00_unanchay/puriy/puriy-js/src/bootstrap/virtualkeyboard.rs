pub(crate) const VIRTUALKEYBOARD_BOOTSTRAP: &str = r#"
// Fase 7.170 — Virtual Keyboard API (`navigator.virtualKeyboard`).
// Da control sobre el teclado virtual on-screen (móvil/tablet): apps que quieren pintar
// debajo del teclado (`overlaysContent = true` cede el manejo del viewport al autor) y
// saber su geometría para reposicionar la UI (un chat que sube el input, un editor que
// scrollea el cursor a la vista). Es un EventTarget con `overlaysContent` (bool),
// `boundingRect` (DOMRect del teclado — todo-cero cuando está oculto) y `show()`/`hide()`.
// Evento `geometrychange` + `ongeometrychange` cuando el teclado aparece/desaparece. El
// chrome reporta la geometría real con `__puriy_virtual_keyboard_geometry(x,y,w,h)`
// (PENDIENTE — sin teclado on-screen nativo en desktop todavía).
(function() {
    if (globalThis.VirtualKeyboard != null) return;
    var nav = globalThis.navigator;
    if (nav == null) return;
    if (typeof globalThis.EventTarget !== 'function') return;  // requiere Fase 7.76

    function rect(x, y, w, h) {
        return { x: x, y: y, width: w, height: h, top: y, left: x,
                 right: x + w, bottom: y + h, toJSON: function() {
            return { x: x, y: y, width: w, height: h, top: y, left: x, right: x + w, bottom: y + h };
        } };
    }

    function VirtualKeyboard() {
        globalThis.EventTarget.call(this);
        this.overlaysContent = false;
        this.boundingRect = rect(0, 0, 0, 0);
        this.ongeometrychange = null;
    }
    VirtualKeyboard.prototype = Object.create(globalThis.EventTarget.prototype);
    VirtualKeyboard.prototype.constructor = VirtualKeyboard;

    VirtualKeyboard.prototype.show = function() {
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'virtualkeyboard', value: 'show' });
    };
    VirtualKeyboard.prototype.hide = function() {
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'virtualkeyboard', value: 'hide' });
    };
    globalThis.VirtualKeyboard = VirtualKeyboard;

    var vk = new VirtualKeyboard();
    nav.virtualKeyboard = vk;

    // El chrome reporta la geometría real del teclado (x,y,w,h). w/h en 0 = oculto.
    globalThis.__puriy_virtual_keyboard_geometry = function(x, y, w, h) {
        vk.boundingRect = rect(+x || 0, +y || 0, +w || 0, +h || 0);
        var ev = new globalThis.Event('geometrychange', {});
        ev.target = vk;
        if (typeof vk.ongeometrychange === 'function') {
            try { vk.ongeometrychange.call(vk, ev); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
        vk.dispatchEvent(ev);
        return true;
    };
    void 0;
})();
"#;
