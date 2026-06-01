// Fase 7.172 — modernización del runtime de lenguaje (ES2023/2024).
//
// El blob QuickJS embebido es anterior a varias adiciones de la stdlib que
// los sites modernos ya usan sin transpilar. Este módulo las polyfillea, todas
// **feature-detectadas** (`if (typeof X !== 'function')`): si el motor ya las
// trae nativas, no se tocan. Y todas en estilo Promise-encadenada PURA — sin
// `async`/`await` en el source — para no arriesgar un SyntaxError en el
// bootstrap si el build de QuickJS no soporta la sintaxis async (que rompería
// el init de TODO el runtime, no sólo esta feature).
//
// Cubre:
//   - `Promise.withResolvers()`           (ES2024)
//   - `Array.fromAsync(items, mapFn?, t?)` (ES2024)
//   - `Object.groupBy(items, cb)`          (ES2024)
//   - `Map.groupBy(items, cb)`             (ES2024)
//
// Depende sólo de primitivas ya presentes (Promise, Symbol.iterator,
// Symbol.asyncIterator — éste último ya usado por streams.rs).
pub(crate) const LANG_MODERN_BOOTSTRAP: &str = r#"
// --- Promise.withResolvers() --------------------------------------------
// Devuelve { promise, resolve, reject } sin la danza del executor. Usa `this`
// como constructor para que subclases de Promise hereden el helper.
if (typeof Promise.withResolvers !== 'function') {
    Object.defineProperty(Promise, 'withResolvers', {
        configurable: true, writable: true,
        value: function withResolvers() {
            var resolve, reject;
            var promise = new this(function(res, rej) { resolve = res; reject = rej; });
            return { promise: promise, resolve: resolve, reject: reject };
        }
    });
}

// --- Array.fromAsync(items, mapFn?, thisArg?) ----------------------------
// Como Array.from pero awaiteando: soporta async-iterables, sync-iterables
// (awaiteando cada valor) y array-likes. Devuelve siempre una Promise.
// Implementado con encadenado manual de Promesas (sin async/await syntax).
if (typeof Array.fromAsync !== 'function') {
    Object.defineProperty(Array, 'fromAsync', {
        configurable: true, writable: true,
        value: function fromAsync(items, mapFn, thisArg) {
            var C = this;
            return new Promise(function(resolve, reject) {
                try {
                    if (items === null || items === undefined) {
                        throw new TypeError('Array.fromAsync: se requiere un iterable o array-like');
                    }
                    if (mapFn !== undefined && typeof mapFn !== 'function') {
                        throw new TypeError('Array.fromAsync: mapFn no es una función');
                    }
                    var result = (typeof C === 'function') ? new C() : [];
                    var i = 0;
                    var iter = null;
                    var asyncCtor = items[Symbol.asyncIterator];
                    if (typeof asyncCtor === 'function') {
                        iter = asyncCtor.call(items);
                    } else {
                        var syncCtor = items[Symbol.iterator];
                        if (typeof syncCtor === 'function') {
                            iter = syncCtor.call(items);
                        }
                    }
                    var put = function(val, next) {
                        // Awaitea el valor crudo, luego (si hay) el mapFn.
                        return Promise.resolve(val).then(function(v) {
                            if (mapFn) {
                                var idx = i;
                                return Promise.resolve(mapFn.call(thisArg, v, idx)).then(function(mv) {
                                    result[i++] = mv; return next();
                                });
                            }
                            result[i++] = v; return next();
                        });
                    };
                    if (iter) {
                        var step = function() {
                            // next() de un async-iterator devuelve Promise; de uno
                            // sync, un objeto plano — Promise.resolve unifica ambos.
                            return Promise.resolve(iter.next()).then(function(res) {
                                if (res.done) { result.length = i; resolve(result); return; }
                                return put(res.value, step);
                            });
                        };
                        Promise.resolve().then(step).catch(reject);
                    } else {
                        // Array-like: índices 0..length-1, awaiteando cada slot.
                        var len = items.length >>> 0;
                        var stepAL = function() {
                            if (i >= len) { result.length = len; resolve(result); return; }
                            return put(items[i], stepAL);
                        };
                        Promise.resolve().then(stepAL).catch(reject);
                    }
                } catch (e) { reject(e); }
            });
        }
    });
}

// --- Object.groupBy(items, callback) -------------------------------------
// Agrupa en un objeto de prototipo nulo: claves = retorno del callback
// (coercionado a property key), valores = arrays de items.
if (typeof Object.groupBy !== 'function') {
    Object.defineProperty(Object, 'groupBy', {
        configurable: true, writable: true,
        value: function groupBy(items, callbackfn) {
            if (typeof callbackfn !== 'function') {
                throw new TypeError('Object.groupBy: el callback no es una función');
            }
            if (items === null || items === undefined) {
                throw new TypeError('Object.groupBy: items es null o undefined');
            }
            var obj = Object.create(null);
            var i = 0;
            var has = Object.prototype.hasOwnProperty;
            for (var item of items) {
                var key = callbackfn(item, i++);
                if (has.call(obj, key)) { obj[key].push(item); }
                else { obj[key] = [item]; }
            }
            return obj;
        }
    });
}

// --- Map.groupBy(items, callback) ----------------------------------------
// Como Object.groupBy pero con un Map: las claves comparan por SameValueZero,
// así que se puede agrupar por identidad de objeto (no sólo por string).
if (typeof Map.groupBy !== 'function') {
    Object.defineProperty(Map, 'groupBy', {
        configurable: true, writable: true,
        value: function groupBy(items, callbackfn) {
            if (typeof callbackfn !== 'function') {
                throw new TypeError('Map.groupBy: el callback no es una función');
            }
            if (items === null || items === undefined) {
                throw new TypeError('Map.groupBy: items es null o undefined');
            }
            var map = new Map();
            var i = 0;
            for (var item of items) {
                var key = callbackfn(item, i++);
                var group = map.get(key);
                if (group !== undefined) { group.push(item); }
                else { map.set(key, [item]); }
            }
            return map;
        }
    });
}
"#;
