//! Tabla US-layout `key → keyCode` legacy (`__puriy_key_to_keycode`) +
//! sus dos lookups (`__puriy_key_named_codes`, `__puriy_key_punct_codes`).
//!
//! Extraído del `dom_events.rs` del frente `events` para que
//! `keyboard_events.rs` (que computa `KeyboardEvent.keyCode`) tenga su
//! dependencia sin arrastrar el resto de aquel `dom_events`. Debe cargarse
//! **antes** de `keyboard_events`. Idempotente (guard por existencia).
//!
//! `keyCode` está deprecado en el spec, pero scripts viejos lo leen; lo
//! derivamos del `key`/`code` canónico (no hay scancodes físicos acá).

pub(crate) const KEYCODE_TABLE_BOOTSTRAP: &str = r#"
(function(){
  if (typeof globalThis.__puriy_key_to_keycode === 'function') return;

  globalThis.__puriy_key_named_codes = {
      'Enter': 13, 'Escape': 27, 'Tab': 9, 'Backspace': 8, 'Delete': 46,
      'ArrowLeft': 37, 'ArrowUp': 38, 'ArrowRight': 39, 'ArrowDown': 40,
      'Shift': 16, 'Control': 17, 'Alt': 18, 'Meta': 91, 'CapsLock': 20,
      'Home': 36, 'End': 35, 'PageUp': 33, 'PageDown': 34, 'Insert': 45,
      'Clear': 12, 'Pause': 19, 'PrintScreen': 44, 'ContextMenu': 93,
      'NumLock': 144, 'ScrollLock': 145
  };
  globalThis.__puriy_key_punct_codes = {
      ';': 186, ':': 186, '=': 187, '+': 187, ',': 188, '<': 188,
      '-': 189, '_': 189, '.': 190, '>': 190, '/': 191, '?': 191,
      '`': 192, '~': 192, '[': 219, '{': 219, '\\': 220, '|': 220,
      ']': 221, '}': 221, "'": 222, '"': 222
  };
  globalThis.__puriy_key_to_keycode = function(key, code) {
      if (typeof key !== 'string' || key === '') return 0;
      if (key.length === 1) {
          var up = key.toUpperCase();
          var cc = up.charCodeAt(0);
          if (cc >= 65 && cc <= 90) return cc;
          if (key >= '0' && key <= '9') return key.charCodeAt(0);
          if (key === ' ') return 32;
          var p = globalThis.__puriy_key_punct_codes[key];
          if (p !== undefined) return p;
          return key.charCodeAt(0);
      }
      var n = globalThis.__puriy_key_named_codes[key];
      if (n !== undefined) return n;
      var fm = /^F(\d{1,2})$/.exec(key);
      if (fm) { var fn = parseInt(fm[1], 10); if (fn >= 1 && fn <= 24) return 111 + fn; }
      if (typeof code === 'string') {
          var dm = /^Numpad(\d)$/.exec(code);
          if (dm) return 96 + parseInt(dm[1], 10);
      }
      return 0;
  };
})();
"#;
