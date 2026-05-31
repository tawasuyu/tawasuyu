pub(crate) const ANIMATIONS_BOOTSTRAP: &str = r#"
// Fase 7.127 — Web Animations API (`element.animate()` + `Animation`). Apps la usan para
// animar opacidad/transform/color desde JS sin CSS keyframes, con control imperativo
// (play/pause/reverse/finish/cancel) y promesas (`finished`/`ready`). El motor no interpola
// estilos todavía (eso requiere que el chrome lea los keyframes y repinte por frame); acá
// modelamos la máquina de estados de timing y el ciclo de vida de las promesas, que es lo
// que el 90% del código JS observa. `el.animate(keyframes, options)` (método en
// __puriy_make_element) crea una `Animation` en `playState: 'running'` y agenda el fin vía
// setTimeout(duration) (Fase timers). `finish()`/`cancel()` resuelven/rechazan `finished`.
// Publica `kind: 'animate'` (value = id del elemento) para que el chrome, cuando sepa
// interpolar, repinte; hoy es informativo.
(function() {
    if (globalThis.Animation != null) return;

    globalThis.__puriy_anim_next_id = globalThis.__puriy_anim_next_id || 1;

    function normalizeOptions(options) {
        if (options == null) return { duration: 0, delay: 0, iterations: 1 };
        if (typeof options === 'number') return { duration: options, delay: 0, iterations: 1 };
        return {
            duration: (typeof options.duration === 'number') ? options.duration : 0,
            delay: (typeof options.delay === 'number') ? options.delay : 0,
            iterations: (options.iterations != null) ? options.iterations : 1
        };
    }

    function KeyframeEffect(target, keyframes, options) {
        this.target = target || null;
        this._keyframes = keyframes || [];
        this._timing = normalizeOptions(options);
    }
    KeyframeEffect.prototype.getKeyframes = function() { return this._keyframes.slice(); };
    KeyframeEffect.prototype.getTiming = function() {
        return { duration: this._timing.duration, delay: this._timing.delay, iterations: this._timing.iterations };
    };
    globalThis.KeyframeEffect = KeyframeEffect;

    function Animation(effect) {
        this.id = '';
        this.effect = effect || null;
        this.playState = 'idle';
        this.currentTime = 0;
        this.playbackRate = 1;
        this.startTime = null;
        this.onfinish = null;
        this.oncancel = null;
        this._timer = null;
        var self = this;
        this.finished = new Promise(function(resolve, reject) {
            self._finishResolve = resolve; self._finishReject = reject;
        });
        // El navegador real resuelve `ready` en el próximo frame; acá ya.
        this.ready = Promise.resolve(this);
        // Evita unhandled rejection si nadie observa `finished` y se cancela.
        if (typeof this.finished.catch === 'function') this.finished.catch(function() {});
    }
    Animation.prototype._duration = function() {
        return (this.effect && this.effect._timing) ? (this.effect._timing.duration | 0) : 0;
    };
    Animation.prototype._fire = function(type) {
        if (typeof this['on' + type] === 'function') {
            try { this['on' + type].call(this, { type: type }); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
    };
    Animation.prototype.play = function() {
        if (this.playState === 'finished') { this.currentTime = 0; }
        this.playState = 'running';
        var self = this;
        if (this._timer != null && typeof globalThis.clearTimeout === 'function') {
            globalThis.clearTimeout(this._timer);
        }
        // Agenda el fin natural. duration 0 → fin inmediato en el próximo tick.
        this._timer = globalThis.setTimeout(function() { self.finish(); }, this._duration());
        return this;
    };
    Animation.prototype.pause = function() {
        this.playState = 'paused';
        if (this._timer != null && typeof globalThis.clearTimeout === 'function') {
            globalThis.clearTimeout(this._timer); this._timer = null;
        }
    };
    Animation.prototype.reverse = function() {
        this.playbackRate = -this.playbackRate;
        return this.play();
    };
    Animation.prototype.finish = function() {
        if (this.playState === 'finished') return;
        this.playState = 'finished';
        this.currentTime = this._duration();
        if (this._timer != null && typeof globalThis.clearTimeout === 'function') {
            globalThis.clearTimeout(this._timer); this._timer = null;
        }
        this._fire('finish');
        if (this._finishResolve) { this._finishResolve(this); this._finishResolve = null; }
    };
    Animation.prototype.cancel = function() {
        var wasActive = (this.playState !== 'idle');
        this.playState = 'idle';
        this.currentTime = 0;
        if (this._timer != null && typeof globalThis.clearTimeout === 'function') {
            globalThis.clearTimeout(this._timer); this._timer = null;
        }
        if (wasActive) this._fire('cancel');
        if (this._finishReject) {
            this._finishReject(new globalThis.DOMException('Animación cancelada', 'AbortError'));
            this._finishReject = null;
        }
    };
    globalThis.Animation = Animation;

    // Llamado por `el.animate()` (definido en __puriy_make_element).
    globalThis.__puriy_animate = function(elementId, keyframes, options) {
        var target = (globalThis.__puriy_elements && globalThis.__puriy_elements[elementId]) || null;
        var effect = new KeyframeEffect(target, keyframes, options);
        var anim = new Animation(effect);
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'animate', value: String(elementId) });
        anim.play();
        return anim;
    };
    void 0;
})();
"#;
