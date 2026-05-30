pub(crate) const MEDIA_SESSION_BOOTSTRAP: &str = r#"
// Fase 7.107 — `navigator.mediaSession` (Media Session API). Los reproductores la
// usan para mandar metadata (título/artista/álbum/carátula) y estado a los
// controles de medios del SO (teclas play/pause del teclado, lock screen, etc.),
// y para recibir esas acciones de vuelta. Host-driven en ambos sentidos: setear
// `metadata`/`playbackState`/un handler publica una mutación al chrome vía
// `__puriy_dirty` (mismo canal que wakelock 7.103); el chrome dispara una tecla
// multimedia llamando `__puriy_media_session_action(action, details)`, que invoca
// el handler registrado. `MediaMetadata` es el constructor de metadata.
// `setPositionState({duration, position, playbackRate})` guarda la barra de
// progreso (validación de duration como en el spec).
(function() {
    var nav = globalThis.navigator = globalThis.navigator || {};
    if (nav.mediaSession != null) return;

    function MediaMetadata(init) {
        init = init || {};
        this.title = init.title != null ? String(init.title) : '';
        this.artist = init.artist != null ? String(init.artist) : '';
        this.album = init.album != null ? String(init.album) : '';
        this.artwork = Array.isArray(init.artwork) ? init.artwork.map(function(a) {
            a = a || {};
            return { src: String(a.src != null ? a.src : ''),
                     sizes: String(a.sizes != null ? a.sizes : ''),
                     type: String(a.type != null ? a.type : '') };
        }) : [];
    }
    globalThis.MediaMetadata = MediaMetadata;

    var ACTIONS = ['play','pause','stop','seekbackward','seekforward','seekto',
                   'previoustrack','nexttrack','skipad','togglemicrophone',
                   'togglecamera','hangup'];

    function MediaSession() {
        this._metadata = null;
        this._playbackState = 'none';
        this._handlers = Object.create(null);
        this._position = null;
    }
    Object.defineProperty(MediaSession.prototype, 'metadata', {
        get: function() { return this._metadata; },
        set: function(m) {
            this._metadata = m;
            var summary = m ? { title: m.title, artist: m.artist, album: m.album } : null;
            globalThis.__puriy_dirty.push({
                id: '__window__', kind: 'mediasession-metadata', value: JSON.stringify(summary)
            });
        },
        enumerable: true, configurable: true
    });
    Object.defineProperty(MediaSession.prototype, 'playbackState', {
        get: function() { return this._playbackState; },
        set: function(s) {
            s = String(s);
            if (s !== 'none' && s !== 'paused' && s !== 'playing') return;
            this._playbackState = s;
            globalThis.__puriy_dirty.push({
                id: '__window__', kind: 'mediasession-playbackstate', value: s
            });
        },
        enumerable: true, configurable: true
    });
    MediaSession.prototype.setActionHandler = function(action, handler) {
        action = String(action);
        if (ACTIONS.indexOf(action) < 0) {
            throw new TypeError("mediaSession: acción '" + action + "' no soportada");
        }
        if (handler == null) delete this._handlers[action];
        else this._handlers[action] = handler;
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'mediasession-handler',
            value: action + ':' + (handler == null ? '0' : '1')
        });
    };
    MediaSession.prototype.setPositionState = function(st) {
        if (st == null) { this._position = null; return; }
        var dur = Number(st.duration);
        if (!(dur >= 0)) throw new TypeError('setPositionState: duration inválida');
        this._position = {
            duration: dur,
            playbackRate: st.playbackRate != null ? Number(st.playbackRate) : 1.0,
            position: st.position != null ? Number(st.position) : 0.0
        };
    };
    globalThis.MediaSession = MediaSession;
    nav.mediaSession = new MediaSession();

    // El chrome enruta una tecla/botón multimedia del SO al handler registrado.
    globalThis.__puriy_media_session_action = function(action, details) {
        var h = nav.mediaSession._handlers[String(action)];
        if (typeof h === 'function') {
            try { h(details || { action: String(action) }); }
            catch (e) { globalThis.__puriy_stderr += String(e) + '\n'; }
            return true;
        }
        return false;
    };
})();
"#;
