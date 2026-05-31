pub(crate) const WEBRTC_BOOTSTRAP: &str = r#"
// Fase 7.142 — WebRTC (`RTCPeerConnection`/`RTCDataChannel`/`RTCSessionDescription`/
// `RTCIceCandidate`). Conexiones peer-to-peer (audio/video/datos) con NAT traversal.
// La máquina de señalización y los data channels son JS-puros y funcionales (las apps
// pueden manejar offer/answer/datachannel localmente), pero el transporte real —ICE,
// DTLS, SRTP, el medio— es del chrome (wiring nativo PENDIENTE):
//   · createOffer/createAnswer resuelven con un SDP sintético local (suficiente para el
//     contrato observable; el SDP real con candidatos lo produce el chrome).
//   · setLocalDescription/setRemoteDescription publican kind: 'rtc-local-description' /
//     'rtc-remote-description' y mueven signalingState como manda el spec.
//   · channel.send(...) publica kind: 'rtc-datachannel-send' (value `<pcId> GS <label> GS <data>`).
//   · El chrome empuja candidatos ICE (`__puriy_rtc_ice_candidate(pcId, cand)`), cambios de
//     estado (`__puriy_rtc_state(pcId, kind, value)`), data channels entrantes
//     (`__puriy_rtc_datachannel(pcId, info)`) y mensajes (`__puriy_rtc_datachannel_message(pcId, label, data)`).
(function() {
    if (globalThis.RTCPeerConnection != null) return;
    var GS = String.fromCharCode(0x1D);
    var nextPcId = 1;
    var pcs = {};

    function mix(proto) {
        proto.addEventListener = function(type, fn) {
            (this._listeners[type] = this._listeners[type] || []).push(fn);
        };
        proto.removeEventListener = function(type, fn) {
            var a = this._listeners[type]; if (!a) return;
            var i = a.indexOf(fn); if (i >= 0) a.splice(i, 1);
        };
        proto.dispatchEvent = function(ev) {
            var a = this._listeners[ev.type];
            if (a) { var c = a.slice(); for (var i = 0; i < c.length; i++) c[i].call(this, ev); }
            if (typeof this['on' + ev.type] === 'function') this['on' + ev.type](ev);
            return true;
        };
    }

    // ---- RTCSessionDescription / RTCIceCandidate ----
    function RTCSessionDescription(init) {
        init = init || {};
        this.type = init.type != null ? init.type : null;
        this.sdp = init.sdp != null ? init.sdp : '';
    }
    RTCSessionDescription.prototype.toJSON = function() { return { type: this.type, sdp: this.sdp }; };

    function RTCIceCandidate(init) {
        init = init || {};
        this.candidate = init.candidate != null ? init.candidate : '';
        this.sdpMid = init.sdpMid != null ? init.sdpMid : null;
        this.sdpMLineIndex = init.sdpMLineIndex != null ? init.sdpMLineIndex : null;
        this.usernameFragment = init.usernameFragment != null ? init.usernameFragment : null;
    }
    RTCIceCandidate.prototype.toJSON = function() {
        return { candidate: this.candidate, sdpMid: this.sdpMid,
                 sdpMLineIndex: this.sdpMLineIndex, usernameFragment: this.usernameFragment };
    };

    // ---- RTCDataChannel ----
    function RTCDataChannel(pc, label, opts) {
        opts = opts || {};
        this._pc = pc;
        this.label = label != null ? String(label) : '';
        this.ordered = opts.ordered !== false;
        this.protocol = opts.protocol != null ? opts.protocol : '';
        this.id = opts.id != null ? opts.id : null;
        this.maxPacketLifeTime = opts.maxPacketLifeTime != null ? opts.maxPacketLifeTime : null;
        this.maxRetransmits = opts.maxRetransmits != null ? opts.maxRetransmits : null;
        this.negotiated = !!opts.negotiated;
        this.readyState = 'connecting';
        this.bufferedAmount = 0;
        this.bufferedAmountLowThreshold = 0;
        this.binaryType = 'blob';
        this._listeners = {};
        pc._channels[this.label] = this;
    }
    mix(RTCDataChannel.prototype);
    RTCDataChannel.prototype._open = function() {
        if (this.readyState !== 'connecting') return;
        this.readyState = 'open';
        this.dispatchEvent({ type: 'open' });
    };
    RTCDataChannel.prototype.send = function(data) {
        if (this.readyState === 'closing' || this.readyState === 'closed') {
            throw new globalThis.DOMException('data channel cerrado', 'InvalidStateError');
        }
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'rtc-datachannel-send',
            value: this._pc._id + GS + this.label + GS + String(data)
        });
    };
    RTCDataChannel.prototype.close = function() {
        if (this.readyState === 'closed') return;
        this.readyState = 'closing';
        var self = this;
        Promise.resolve().then(function() {
            self.readyState = 'closed';
            self.dispatchEvent({ type: 'close' });
        });
    };

    // ---- RTCPeerConnection ----
    function RTCPeerConnection(config) {
        this._id = nextPcId++;
        pcs[this._id] = this;
        this._config = config || {};
        this.localDescription = null;
        this.remoteDescription = null;
        this.currentLocalDescription = null;
        this.currentRemoteDescription = null;
        this.signalingState = 'stable';
        this.iceConnectionState = 'new';
        this.iceGatheringState = 'new';
        this.connectionState = 'new';
        this.canTrickleIceCandidates = null;
        this._channels = {};
        this._lastCreated = null;
        this._listeners = {};
    }
    mix(RTCPeerConnection.prototype);

    function synthSdp(type) {
        return 'v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=puriy\r\nt=0 0\r\na=' + type + '\r\n';
    }

    RTCPeerConnection.prototype.createOffer = function() {
        var d = { type: 'offer', sdp: synthSdp('offer') };
        this._lastCreated = d;
        return Promise.resolve(d);
    };
    RTCPeerConnection.prototype.createAnswer = function() {
        var d = { type: 'answer', sdp: synthSdp('answer') };
        this._lastCreated = d;
        return Promise.resolve(d);
    };
    RTCPeerConnection.prototype.setLocalDescription = function(desc) {
        desc = desc || this._lastCreated;
        if (desc == null) return Promise.reject(new globalThis.DOMException('sin descripción', 'InvalidStateError'));
        this.localDescription = new RTCSessionDescription(desc);
        this.currentLocalDescription = this.localDescription;
        if (desc.type === 'offer') this.signalingState = 'have-local-offer';
        else if (desc.type === 'answer' || desc.type === 'pranswer') this.signalingState = 'stable';
        var self = this;
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'rtc-local-description',
            value: self._id + GS + (desc.type || '') + GS + (desc.sdp || '')
        });
        this.dispatchEvent({ type: 'signalingstatechange' });
        return Promise.resolve();
    };
    RTCPeerConnection.prototype.setRemoteDescription = function(desc) {
        if (desc == null) return Promise.reject(new globalThis.DOMException('sin descripción', 'InvalidStateError'));
        this.remoteDescription = new RTCSessionDescription(desc);
        this.currentRemoteDescription = this.remoteDescription;
        if (desc.type === 'offer') this.signalingState = 'have-remote-offer';
        else if (desc.type === 'answer' || desc.type === 'pranswer') this.signalingState = 'stable';
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'rtc-remote-description',
            value: this._id + GS + (desc.type || '') + GS + (desc.sdp || '')
        });
        this.dispatchEvent({ type: 'signalingstatechange' });
        return Promise.resolve();
    };
    RTCPeerConnection.prototype.addIceCandidate = function(cand) {
        if (cand != null) {
            var c = (cand instanceof RTCIceCandidate) ? cand : new RTCIceCandidate(cand);
            globalThis.__puriy_dirty.push({
                id: '__window__', kind: 'rtc-add-ice', value: this._id + GS + (c.candidate || '')
            });
        }
        return Promise.resolve();
    };
    RTCPeerConnection.prototype.createDataChannel = function(label, opts) {
        var ch = new RTCDataChannel(this, label, opts);
        globalThis.__puriy_dirty.push({
            id: '__window__', kind: 'rtc-datachannel-open', value: this._id + GS + ch.label
        });
        // Apertura local (el transporte real la gobierna el chrome; aquí abrimos para
        // que el contrato observable —onopen/send— funcione sin red).
        Promise.resolve().then(function() { ch._open(); });
        return ch;
    };
    RTCPeerConnection.prototype.getConfiguration = function() { return this._config; };
    RTCPeerConnection.prototype.setConfiguration = function(c) { this._config = c || {}; };
    RTCPeerConnection.prototype.getSenders = function() { return []; };
    RTCPeerConnection.prototype.getReceivers = function() { return []; };
    RTCPeerConnection.prototype.getTransceivers = function() { return []; };
    RTCPeerConnection.prototype.addTrack = function() { return {}; };
    RTCPeerConnection.prototype.removeTrack = function() {};
    RTCPeerConnection.prototype.addTransceiver = function() { return {}; };
    RTCPeerConnection.prototype.getStats = function() { return Promise.resolve(new Map()); };
    RTCPeerConnection.prototype.restartIce = function() {
        this.iceGatheringState = 'new';
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'rtc-restart-ice', value: String(this._id) });
    };
    RTCPeerConnection.prototype.close = function() {
        if (this.signalingState === 'closed') return;
        this.signalingState = 'closed';
        this.connectionState = 'closed';
        this.iceConnectionState = 'closed';
        for (var k in this._channels) {
            if (Object.prototype.hasOwnProperty.call(this._channels, k)) {
                this._channels[k].readyState = 'closed';
            }
        }
        globalThis.__puriy_dirty.push({ id: '__window__', kind: 'rtc-close', value: String(this._id) });
    };

    // ---- Hooks del host ----
    globalThis.__puriy_rtc_ice_candidate = function(pcId, candidate) {
        var pc = pcs[pcId]; if (!pc) return false;
        var c = candidate == null ? null : new RTCIceCandidate(candidate);
        pc.dispatchEvent({ type: 'icecandidate', candidate: c });
        return true;
    };
    globalThis.__puriy_rtc_state = function(pcId, kind, value) {
        var pc = pcs[pcId]; if (!pc) return false;
        if (kind === 'connection') { pc.connectionState = value; pc.dispatchEvent({ type: 'connectionstatechange' }); }
        else if (kind === 'ice') { pc.iceConnectionState = value; pc.dispatchEvent({ type: 'iceconnectionstatechange' }); }
        else if (kind === 'gathering') { pc.iceGatheringState = value; pc.dispatchEvent({ type: 'icegatheringstatechange' }); }
        else if (kind === 'signaling') { pc.signalingState = value; pc.dispatchEvent({ type: 'signalingstatechange' }); }
        return true;
    };
    globalThis.__puriy_rtc_datachannel = function(pcId, info) {
        var pc = pcs[pcId]; if (!pc) return false;
        info = info || {};
        var ch = new RTCDataChannel(pc, info.label, info);
        ch.readyState = 'open';
        pc.dispatchEvent({ type: 'datachannel', channel: ch });
        return true;
    };
    globalThis.__puriy_rtc_datachannel_message = function(pcId, label, data) {
        var pc = pcs[pcId]; if (!pc) return false;
        var ch = pc._channels[label]; if (!ch) return false;
        ch.dispatchEvent({ type: 'message', data: data });
        return true;
    };

    globalThis.RTCPeerConnection = RTCPeerConnection;
    globalThis.webkitRTCPeerConnection = RTCPeerConnection;
    globalThis.RTCDataChannel = RTCDataChannel;
    globalThis.RTCSessionDescription = RTCSessionDescription;
    globalThis.RTCIceCandidate = RTCIceCandidate;
    void 0;
})();
"#;
