pub(crate) const GAMEPAD_BOOTSTRAP: &str = r#"
// Fase 7.109 — Gamepad API (`navigator.getGamepads()` + eventos
// `gamepadconnected`/`gamepaddisconnected`). Los juegos polean el estado de los
// mandos cada frame con `getGamepads()` (array de 4 slots, null = vacío). Cada
// Gamepad lleva `index`/`id`/`connected`/`timestamp`/`mapping`/`axes`/`buttons`
// (cada botón `{pressed, touched, value}`). Host-driven: el chrome empuja el
// estado real con `__puriy_set_gamepad(index, state)` (conecta y dispara
// `gamepadconnected` la primera vez, luego sólo actualiza) y desconecta con
// `__puriy_remove_gamepad(index)` (dispara `gamepaddisconnected`). Los eventos
// caen por `__puriy_dispatch_window` (Fase 7.39) con el gamepad pegado.
(function() {
    var nav = globalThis.navigator = globalThis.navigator || {};
    if (nav.getGamepads != null) return;

    globalThis.__puriy_gamepads = globalThis.__puriy_gamepads || [null, null, null, null];

    function buildButtons(raw) {
        return (raw || []).map(function(b) {
            if (typeof b === 'number') return { pressed: b > 0.5, touched: b > 0, value: b };
            b = b || {};
            return { pressed: !!b.pressed, touched: !!b.touched, value: Number(b.value || 0) };
        });
    }
    function makeGamepad(index, state) {
        state = state || {};
        return {
            index: index,
            id: state.id != null ? String(state.id) : 'Gamepad ' + index,
            connected: true,
            timestamp: Number(state.timestamp || 0),
            mapping: state.mapping != null ? String(state.mapping) : 'standard',
            axes: (state.axes || []).map(Number),
            buttons: buildButtons(state.buttons)
        };
    }

    nav.getGamepads = function() {
        return globalThis.__puriy_gamepads.slice();
    };

    globalThis.__puriy_set_gamepad = function(index, state) {
        index = index | 0;
        if (index < 0 || index > 3) return false;
        var nuevo = (globalThis.__puriy_gamepads[index] == null);
        var pad = makeGamepad(index, state);
        globalThis.__puriy_gamepads[index] = pad;
        if (nuevo) globalThis.__puriy_dispatch_window('gamepadconnected', { gamepad: pad });
        return true;
    };
    globalThis.__puriy_remove_gamepad = function(index) {
        index = index | 0;
        var pad = globalThis.__puriy_gamepads[index];
        if (pad == null) return false;
        pad.connected = false;
        globalThis.__puriy_gamepads[index] = null;
        globalThis.__puriy_dispatch_window('gamepaddisconnected', { gamepad: pad });
        return true;
    };
})();
"#;
