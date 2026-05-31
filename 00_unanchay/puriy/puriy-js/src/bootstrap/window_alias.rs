pub(crate) const WINDOW_ALIAS_BOOTSTRAP: &str = r#"
// Fase 7.84 — `window` / `self` (+ parent/top/frames/length) como alias del
// objeto global. QuickJS expone `globalThis` pero NO define `window`: el
// embebido no tiene DOM real. Muchísimo código web asume `window.X`,
// `typeof window !== 'undefined'`, o `self.X` (patrón universal en libs que
// corren igual en window y en worker). Como el global es UN solo objeto, todos
// los alias apuntan a la MISMA referencia — agregar una prop a globalThis se ve
// por window/self automáticamente, y las auto-referencias se cierran solas
// (`window.window === window`, `self.self === self`).
//
// Se carga primero en la cadena de bootstraps para que el resto del runtime y
// el código de usuario vean `window` desde el arranque. Guardas `typeof ... ===
// 'undefined'` por si una fase futura define alguno antes.
if (typeof globalThis.window === 'undefined') globalThis.window = globalThis;
if (typeof globalThis.self === 'undefined') globalThis.self = globalThis;
// Sin iframes la jerarquía de navegación colapsa en el propio global: una
// ventana de tope, sin padre distinto, sin subframes.
if (typeof globalThis.parent === 'undefined') globalThis.parent = globalThis;
if (typeof globalThis.top === 'undefined') globalThis.top = globalThis;
if (typeof globalThis.frames === 'undefined') globalThis.frames = globalThis;
// window.length = número de subframes = 0.
if (typeof globalThis.length === 'undefined') globalThis.length = 0;
"#;
