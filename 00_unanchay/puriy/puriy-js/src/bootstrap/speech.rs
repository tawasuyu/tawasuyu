pub(crate) const SPEECH_BOOTSTRAP: &str = r#"
// Fase 7.114 — Web Speech API, lado síntesis (`speechSynthesis` +
// `SpeechSynthesisUtterance`). Lectores de pantalla y apps de accesibilidad la
// usan para que el navegador "hable" un texto (text-to-speech). El motor no
// tiene síntesis de voz: `speak(utterance)` publica una mutación `kind: 'speak'`
// al chrome (mismo canal que vibrate 7.108), encola el utterance en
// `__puriy_speech_queue` y, vía el event loop (setTimeout 0), dispara los
// eventos del ciclo de vida (`start` luego `end`) sobre el utterance — así el
// código que espera `onend` para encadenar frases funciona sin host real.
// `cancel()` / `pause()` / `resume()` ajustan los flags `speaking`/`paused`/
// `pending` y publican su propia mutación. `getVoices()` devuelve una lista
// (vacía por defecto; el chrome la puebla con `__puriy_set_voices([...])`,
// disparando `voiceschanged`). `SpeechSynthesisUtterance` es un EventTarget
// (Fase 7.76) con `text`/`lang`/`volume`/`rate`/`pitch`/`voice` + handlers.
(function() {
    if (globalThis.speechSynthesis != null) return;

    function SpeechSynthesisUtterance(text) {
        globalThis.EventTarget.call(this);
        this.text = (text != null) ? String(text) : '';
        this.lang = '';
        this.voice = null;
        this.volume = 1;
        this.rate = 1;
        this.pitch = 1;
        this.onstart = null;
        this.onend = null;
        this.onerror = null;
        this.onpause = null;
        this.onresume = null;
        this.onmark = null;
        this.onboundary = null;
    }
    SpeechSynthesisUtterance.prototype = Object.create(globalThis.EventTarget.prototype);
    SpeechSynthesisUtterance.prototype.constructor = SpeechSynthesisUtterance;
    globalThis.SpeechSynthesisUtterance = SpeechSynthesisUtterance;

    globalThis.__puriy_speech_queue = globalThis.__puriy_speech_queue || [];
    var voices = [];

    function fire(target, type, onprop) {
        var ev;
        try { ev = new globalThis.SpeechSynthesisEvent(type); }
        catch (e) { ev = new globalThis.Event(type); }
        if (typeof target[onprop] === 'function') {
            try { target[onprop].call(target, ev); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
        }
        target.dispatchEvent(ev);
    }

    // Evento mínimo del namespace de síntesis (sin offsets reales de audio).
    function SpeechSynthesisEvent(type, init) {
        globalThis.Event.call(this, type, init);
        this.charIndex = (init && init.charIndex != null) ? init.charIndex : 0;
        this.charLength = (init && init.charLength != null) ? init.charLength : 0;
        this.elapsedTime = (init && init.elapsedTime != null) ? init.elapsedTime : 0;
        this.name = (init && init.name != null) ? String(init.name) : '';
        this.utterance = (init && init.utterance != null) ? init.utterance : null;
    }
    SpeechSynthesisEvent.prototype = Object.create(globalThis.Event.prototype);
    SpeechSynthesisEvent.prototype.constructor = SpeechSynthesisEvent;
    globalThis.SpeechSynthesisEvent = SpeechSynthesisEvent;

    function SpeechSynthesis() {
        globalThis.EventTarget.call(this);
        this.pending = false;
        this.speaking = false;
        this.paused = false;
        this.onvoiceschanged = null;
    }
    SpeechSynthesis.prototype = Object.create(globalThis.EventTarget.prototype);
    SpeechSynthesis.prototype.constructor = SpeechSynthesis;

    SpeechSynthesis.prototype.speak = function(utterance) {
        if (!(utterance instanceof SpeechSynthesisUtterance)) {
            throw new TypeError('speak requiere un SpeechSynthesisUtterance');
        }
        var self = this;
        globalThis.__puriy_speech_queue.push(utterance);
        this.pending = true;
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'speak', value: JSON.stringify({
                text: utterance.text, lang: utterance.lang,
                volume: utterance.volume, rate: utterance.rate, pitch: utterance.pitch
            })
        });
        // Simula el ciclo start→end de forma diferida (el host real lo manejaría).
        globalThis.setTimeout(function() {
            var i = globalThis.__puriy_speech_queue.indexOf(utterance);
            if (i < 0) return; // cancelado antes de empezar
            self.pending = false;
            self.speaking = true;
            fire(utterance, 'start', 'onstart');
            globalThis.setTimeout(function() {
                var j = globalThis.__puriy_speech_queue.indexOf(utterance);
                if (j >= 0) globalThis.__puriy_speech_queue.splice(j, 1);
                self.speaking = (globalThis.__puriy_speech_queue.length > 0);
                fire(utterance, 'end', 'onend');
            }, 0);
        }, 0);
        return undefined;
    };
    SpeechSynthesis.prototype.cancel = function() {
        globalThis.__puriy_speech_queue.length = 0;
        this.pending = false;
        this.speaking = false;
        this.paused = false;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'speak-cancel', value: '' });
    };
    SpeechSynthesis.prototype.pause = function() {
        this.paused = true;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'speak-pause', value: '' });
    };
    SpeechSynthesis.prototype.resume = function() {
        this.paused = false;
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'speak-resume', value: '' });
    };
    SpeechSynthesis.prototype.getVoices = function() {
        return voices.slice();
    };
    globalThis.SpeechSynthesis = SpeechSynthesis;

    var synth = new SpeechSynthesis();
    globalThis.speechSynthesis = synth;

    // Hook de ingreso: el chrome puebla la lista de voces disponibles y dispara
    // `voiceschanged` (mismo patrón host-driven que connection 7.89).
    globalThis.__puriy_set_voices = function(list) {
        voices = [];
        if (Array.isArray(list)) {
            for (var i = 0; i < list.length; i++) {
                var v = list[i] || {};
                voices.push({
                    voiceURI: (v.voiceURI != null) ? String(v.voiceURI) : '',
                    name: (v.name != null) ? String(v.name) : '',
                    lang: (v.lang != null) ? String(v.lang) : '',
                    localService: !!v.localService,
                    default: !!v.default
                });
            }
        }
        fire(synth, 'voiceschanged', 'onvoiceschanged');
        return true;
    };
})();
"#;
