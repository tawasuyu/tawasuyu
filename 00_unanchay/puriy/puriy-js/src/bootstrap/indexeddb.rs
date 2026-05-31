pub(crate) const INDEXEDDB_BOOTSTRAP: &str = r#"
// Fase 7.141 — IndexedDB (`indexedDB`, IDBFactory/IDBDatabase/IDBObjectStore/IDBIndex/
// IDBTransaction/IDBRequest/IDBCursor/IDBKeyRange). Base de datos transaccional clave-valor,
// la pieza de almacenamiento estructurado más usada de la plataforma. 100% JS-puro y funcional
// (no necesita chrome): el store vive en memoria bajo `__puriy_idb_dbs` y persiste mientras viva
// el runtime. Las peticiones (`IDBRequest`) disparan `success`/`error` de forma asíncrona vía
// microtask — el harness drena microtasks tras cada eval, así que `onsuccess` corre antes de
// observar el resultado, igual que en un navegador dentro de la misma vuelta de evento.
// Divergencia: no hay persistencia a disco (es in-memory) y el versionado de upgrade es local.
(function() {
    if (globalThis.indexedDB != null) return;
    var DBS = globalThis.__puriy_idb_dbs = globalThis.__puriy_idb_dbs || {};

    function micro(fn) { Promise.resolve().then(fn); }

    // ---- Comparación de claves al estilo IndexedDB: number < string < array ----
    function typeRank(k) {
        if (typeof k === 'number') return 1;
        if (typeof k === 'string') return 2;
        if (Array.isArray(k)) return 3;
        return 0;
    }
    function cmpKeys(a, b) {
        var ra = typeRank(a), rb = typeRank(b);
        if (ra !== rb) return ra < rb ? -1 : 1;
        if (ra === 1) return a < b ? -1 : (a > b ? 1 : 0);
        if (ra === 2) return a < b ? -1 : (a > b ? 1 : 0);
        if (ra === 3) {
            var n = Math.min(a.length, b.length);
            for (var i = 0; i < n; i++) { var c = cmpKeys(a[i], b[i]); if (c !== 0) return c; }
            return a.length < b.length ? -1 : (a.length > b.length ? 1 : 0);
        }
        return 0;
    }

    function extractKey(keyPath, value) {
        if (keyPath == null) return undefined;
        if (Array.isArray(keyPath)) {
            var arr = [];
            for (var i = 0; i < keyPath.length; i++) arr.push(extractKey(keyPath[i], value));
            return arr;
        }
        var parts = String(keyPath).split('.');
        var cur = value;
        for (var j = 0; j < parts.length; j++) {
            if (cur == null) return undefined;
            cur = cur[parts[j]];
        }
        return cur;
    }
    function setKeyPath(keyPath, value, key) {
        // sólo soporta keyPath simple/dotted para inyectar la clave autogenerada
        var parts = String(keyPath).split('.');
        var cur = value;
        for (var i = 0; i < parts.length - 1; i++) {
            if (cur[parts[i]] == null) cur[parts[i]] = {};
            cur = cur[parts[i]];
        }
        cur[parts[parts.length - 1]] = key;
    }

    // ---- IDBKeyRange ----
    function IDBKeyRange(lower, upper, lowerOpen, upperOpen) {
        this.lower = lower; this.upper = upper;
        this.lowerOpen = !!lowerOpen; this.upperOpen = !!upperOpen;
    }
    IDBKeyRange.prototype.includes = function(key) {
        if (this.lower !== undefined) {
            var c = cmpKeys(key, this.lower);
            if (c < 0 || (c === 0 && this.lowerOpen)) return false;
        }
        if (this.upper !== undefined) {
            var d = cmpKeys(key, this.upper);
            if (d > 0 || (d === 0 && this.upperOpen)) return false;
        }
        return true;
    };
    IDBKeyRange.only = function(v) { return new IDBKeyRange(v, v, false, false); };
    IDBKeyRange.lowerBound = function(v, open) { return new IDBKeyRange(v, undefined, open, false); };
    IDBKeyRange.upperBound = function(v, open) { return new IDBKeyRange(undefined, v, false, open); };
    IDBKeyRange.bound = function(l, u, lo, uo) { return new IDBKeyRange(l, u, lo, uo); };
    function matchRange(range, key) {
        if (range == null) return true;
        if (range instanceof IDBKeyRange) return range.includes(key);
        return cmpKeys(key, range) === 0; // clave literal
    }

    // ---- IDBRequest ----
    function IDBRequest(source, transaction) {
        this.result = undefined; this.error = null;
        this.source = source || null; this.transaction = transaction || null;
        this.readyState = 'pending';
        this.onsuccess = null; this.onerror = null;
        this._listeners = {};
    }
    IDBRequest.prototype.addEventListener = function(t, fn) {
        (this._listeners[t] = this._listeners[t] || []).push(fn);
    };
    IDBRequest.prototype.removeEventListener = function(t, fn) {
        var a = this._listeners[t]; if (!a) return;
        var i = a.indexOf(fn); if (i >= 0) a.splice(i, 1);
    };
    IDBRequest.prototype._dispatch = function(type) {
        var ev = { type: type, target: this };
        var a = this._listeners[type];
        if (a) { var c = a.slice(); for (var i = 0; i < c.length; i++) c[i].call(this, ev); }
        var h = this['on' + type];
        if (typeof h === 'function') h.call(this, ev);
    };
    function succeed(req, value) {
        micro(function() {
            req.readyState = 'done'; req.result = value;
            req._dispatch('success');
            if (req.transaction) req.transaction._maybeComplete();
        });
    }
    function fail(req, name, msg) {
        micro(function() {
            req.readyState = 'done';
            req.error = new globalThis.DOMException(msg || 'idb error', name || 'DataError');
            req._dispatch('error');
        });
    }
    function IDBOpenDBRequest(name) { IDBRequest.call(this, null, null); this.onupgradeneeded = null; this.onblocked = null; }
    IDBOpenDBRequest.prototype = Object.create(IDBRequest.prototype);

    // ---- IDBCursor ----
    function IDBCursor(req, items, idx, withValue) {
        this._req = req; this._items = items; this._i = idx; this._withValue = withValue;
        var it = items[idx];
        this.key = it.key; this.primaryKey = it.primaryKey;
        if (withValue) this.value = it.value;
    }
    IDBCursor.prototype.continue = function() {
        var self = this;
        micro(function() {
            self._i++;
            if (self._i >= self._items.length) {
                self._req.result = null; self._req._dispatch('success');
            } else {
                var it = self._items[self._i];
                self.key = it.key; self.primaryKey = it.primaryKey;
                if (self._withValue) self.value = it.value;
                self._req.result = self; self._req._dispatch('success');
            }
        });
    };
    IDBCursor.prototype.advance = function(n) {
        var self = this;
        micro(function() {
            self._i += n;
            if (self._i >= self._items.length) { self._req.result = null; self._req._dispatch('success'); }
            else {
                var it = self._items[self._i];
                self.key = it.key; self.primaryKey = it.primaryKey;
                if (self._withValue) self.value = it.value;
                self._req.result = self; self._req._dispatch('success');
            }
        });
    };
    IDBCursor.prototype.update = function(value) {
        var store = this._req.source._store ? this._req.source._store : this._req.source;
        return store._putRecord(value, this.primaryKey, false);
    };
    IDBCursor.prototype.delete = function() {
        var store = this._req.source._store ? this._req.source._store : this._req.source;
        return store._deleteKey(this.primaryKey);
    };

    // ---- IDBIndex ----
    function IDBIndex(store, meta) {
        this._store = store; this._meta = meta;
        this.name = meta.name; this.keyPath = meta.keyPath;
        this.unique = !!meta.unique; this.multiEntry = !!meta.multiEntry;
        this.objectStore = store;
    }
    IDBIndex.prototype._scan = function(range) {
        var out = [], recs = this._store._data.records;
        for (var i = 0; i < recs.length; i++) {
            var ik = extractKey(this.keyPath, recs[i].value);
            if (ik === undefined) continue;
            if (this.multiEntry && Array.isArray(ik)) {
                for (var j = 0; j < ik.length; j++)
                    if (matchRange(range, ik[j])) out.push({ key: ik[j], primaryKey: recs[i].key, value: recs[i].value });
            } else if (matchRange(range, ik)) {
                out.push({ key: ik, primaryKey: recs[i].key, value: recs[i].value });
            }
        }
        out.sort(function(a, b) { return cmpKeys(a.key, b.key); });
        return out;
    };
    IDBIndex.prototype.get = function(range) {
        var req = new IDBRequest(this, this._store._tx);
        var hits = this._scan(range);
        succeed(req, hits.length ? hits[0].value : undefined);
        return req;
    };
    IDBIndex.prototype.getKey = function(range) {
        var req = new IDBRequest(this, this._store._tx);
        var hits = this._scan(range);
        succeed(req, hits.length ? hits[0].primaryKey : undefined);
        return req;
    };
    IDBIndex.prototype.getAll = function(range) {
        var req = new IDBRequest(this, this._store._tx);
        var hits = this._scan(range), out = [];
        for (var i = 0; i < hits.length; i++) out.push(hits[i].value);
        succeed(req, out);
        return req;
    };
    IDBIndex.prototype.count = function(range) {
        var req = new IDBRequest(this, this._store._tx);
        succeed(req, this._scan(range).length);
        return req;
    };
    IDBIndex.prototype.openCursor = function(range) {
        var req = new IDBRequest(this, this._store._tx);
        var hits = this._scan(range);
        if (!hits.length) { succeed(req, null); return req; }
        micro(function() {
            req.readyState = 'done';
            req.result = new IDBCursor(req, hits, 0, true);
            req._dispatch('success');
        });
        return req;
    };

    // ---- IDBObjectStore ----
    function IDBObjectStore(tx, name) {
        this._tx = tx; this._data = tx._db._stores[name];
        this.name = name; this.keyPath = this._data.keyPath;
        this.autoIncrement = !!this._data.autoIncrement;
        this.transaction = tx;
    }
    Object.defineProperty(IDBObjectStore.prototype, 'indexNames', {
        get: function() { return Object.keys(this._data.indexes); }
    });
    IDBObjectStore.prototype._sorted = function() {
        return this._data.records.slice().sort(function(a, b) { return cmpKeys(a.key, b.key); });
    };
    IDBObjectStore.prototype._find = function(key) {
        var r = this._data.records;
        for (var i = 0; i < r.length; i++) if (cmpKeys(r[i].key, key) === 0) return i;
        return -1;
    };
    IDBObjectStore.prototype._putRecord = function(value, explicitKey, noOverwrite) {
        var req = new IDBRequest(this, this._tx);
        var key = explicitKey;
        if (key === undefined) {
            if (this._data.keyPath != null) key = extractKey(this._data.keyPath, value);
            if (key === undefined && this._data.autoIncrement) {
                key = this._data.keyGen++;
                if (this._data.keyPath != null) setKeyPath(this._data.keyPath, value, key);
            }
        }
        if (key === undefined) { fail(req, 'DataError', 'falta clave'); return req; }
        var idx = this._find(key);
        if (idx >= 0 && noOverwrite) { fail(req, 'ConstraintError', 'clave duplicada'); return req; }
        if (idx >= 0) this._data.records[idx].value = value;
        else this._data.records.push({ key: key, value: value });
        if (typeof key === 'number' && key >= this._data.keyGen) this._data.keyGen = key + 1;
        succeed(req, key);
        return req;
    };
    IDBObjectStore.prototype.add = function(value, key) { return this._putRecord(value, key, true); };
    IDBObjectStore.prototype.put = function(value, key) { return this._putRecord(value, key, false); };
    IDBObjectStore.prototype.get = function(key) {
        var req = new IDBRequest(this, this._tx);
        var i = this._find(key);
        succeed(req, i >= 0 ? this._data.records[i].value : undefined);
        return req;
    };
    IDBObjectStore.prototype.getKey = function(key) {
        var req = new IDBRequest(this, this._tx);
        var i = this._find(key);
        succeed(req, i >= 0 ? this._data.records[i].key : undefined);
        return req;
    };
    IDBObjectStore.prototype.getAll = function(range) {
        var req = new IDBRequest(this, this._tx), out = [], s = this._sorted();
        for (var i = 0; i < s.length; i++) if (matchRange(range, s[i].key)) out.push(s[i].value);
        succeed(req, out);
        return req;
    };
    IDBObjectStore.prototype.getAllKeys = function(range) {
        var req = new IDBRequest(this, this._tx), out = [], s = this._sorted();
        for (var i = 0; i < s.length; i++) if (matchRange(range, s[i].key)) out.push(s[i].key);
        succeed(req, out);
        return req;
    };
    IDBObjectStore.prototype.count = function(range) {
        var req = new IDBRequest(this, this._tx), n = 0, s = this._sorted();
        for (var i = 0; i < s.length; i++) if (matchRange(range, s[i].key)) n++;
        succeed(req, n);
        return req;
    };
    IDBObjectStore.prototype._deleteKey = function(key) {
        var req = new IDBRequest(this, this._tx);
        var i = this._find(key);
        if (i >= 0) this._data.records.splice(i, 1);
        succeed(req, undefined);
        return req;
    };
    IDBObjectStore.prototype.delete = function(key) {
        if (key instanceof IDBKeyRange) {
            var req = new IDBRequest(this, this._tx), recs = this._data.records, kept = [];
            for (var i = 0; i < recs.length; i++) if (!key.includes(recs[i].key)) kept.push(recs[i]);
            this._data.records = kept;
            succeed(req, undefined);
            return req;
        }
        return this._deleteKey(key);
    };
    IDBObjectStore.prototype.clear = function() {
        var req = new IDBRequest(this, this._tx);
        this._data.records = [];
        succeed(req, undefined);
        return req;
    };
    IDBObjectStore.prototype.openCursor = function(range) {
        var req = new IDBRequest(this, this._tx);
        var s = this._sorted(), items = [];
        for (var i = 0; i < s.length; i++)
            if (matchRange(range, s[i].key)) items.push({ key: s[i].key, primaryKey: s[i].key, value: s[i].value });
        if (!items.length) { succeed(req, null); return req; }
        micro(function() {
            req.readyState = 'done';
            req.result = new IDBCursor(req, items, 0, true);
            req._dispatch('success');
        });
        return req;
    };
    IDBObjectStore.prototype.createIndex = function(name, keyPath, opts) {
        opts = opts || {};
        this._data.indexes[name] = { name: name, keyPath: keyPath, unique: !!opts.unique, multiEntry: !!opts.multiEntry };
        return new IDBIndex(this, this._data.indexes[name]);
    };
    IDBObjectStore.prototype.deleteIndex = function(name) { delete this._data.indexes[name]; };
    IDBObjectStore.prototype.index = function(name) {
        var m = this._data.indexes[name];
        if (!m) throw new globalThis.DOMException('índice inexistente: ' + name, 'NotFoundError');
        return new IDBIndex(this, m);
    };

    // ---- IDBTransaction ----
    function IDBTransaction(db, names, mode) {
        this._db = db; this.mode = mode || 'readonly'; this.db = db._facade;
        this._names = names; this._stores = {};
        this.objectStoreNames = names.slice();
        this.error = null;
        this.oncomplete = null; this.onerror = null; this.onabort = null;
        this._listeners = {};
        this._completed = false;
    }
    IDBTransaction.prototype.addEventListener = IDBRequest.prototype.addEventListener;
    IDBTransaction.prototype.removeEventListener = IDBRequest.prototype.removeEventListener;
    IDBTransaction.prototype._dispatch = IDBRequest.prototype._dispatch;
    IDBTransaction.prototype.objectStore = function(name) {
        if (this._names.indexOf(name) < 0)
            throw new globalThis.DOMException('store fuera de la transacción: ' + name, 'NotFoundError');
        if (!this._stores[name]) this._stores[name] = new IDBObjectStore(this, name);
        return this._stores[name];
    };
    IDBTransaction.prototype._maybeComplete = function() {
        if (this._completed) return;
        this._completed = true;
        var self = this;
        micro(function() { self._dispatch('complete'); });
    };
    IDBTransaction.prototype.abort = function() {
        this._completed = true;
        this._dispatch('abort');
    };
    IDBTransaction.prototype.commit = function() { this._maybeComplete(); };

    // ---- IDBDatabase ----
    function IDBDatabase(name) {
        this._name = name; this._stores = {};
        this._meta = DBS[name];
        this.version = this._meta.version;
        this.name = name;
        this._facade = this;
        this.onversionchange = null; this.onclose = null;
        // copia viva: los stores apuntan al meta global
        this._stores = this._meta.stores;
    }
    Object.defineProperty(IDBDatabase.prototype, 'objectStoreNames', {
        get: function() { return Object.keys(this._stores); }
    });
    IDBDatabase.prototype.createObjectStore = function(name, opts) {
        opts = opts || {};
        this._stores[name] = {
            keyPath: (opts.keyPath != null ? opts.keyPath : null),
            autoIncrement: !!opts.autoIncrement,
            keyGen: 1, records: [], indexes: {}
        };
        // devuelve un store ligado a una transacción de upgrade efímera
        var tx = new IDBTransaction(this, [name], 'versionchange');
        return tx.objectStore(name);
    };
    IDBDatabase.prototype.deleteObjectStore = function(name) { delete this._stores[name]; };
    IDBDatabase.prototype.transaction = function(names, mode) {
        if (typeof names === 'string') names = [names];
        return new IDBTransaction(this, names.slice(), mode || 'readonly');
    };
    IDBDatabase.prototype.close = function() { if (typeof this.onclose === 'function') this.onclose(); };

    // ---- IDBFactory ----
    var indexedDB = {
        open: function(name, version) {
            var req = new IDBOpenDBRequest(name);
            var existing = DBS[name];
            var oldVersion = existing ? existing.version : 0;
            var newVersion = (version != null) ? version : (oldVersion || 1);
            if (!existing) DBS[name] = { version: 0, stores: {} };
            micro(function() {
                var db = new IDBDatabase(name);
                if (newVersion > DBS[name].version) {
                    DBS[name].version = newVersion;
                    db.version = newVersion;
                    var tx = new IDBTransaction(db, Object.keys(db._stores), 'versionchange');
                    req.result = db; req.transaction = tx; req.readyState = 'done';
                    req._dispatch('upgradeneeded');
                    req.transaction = null;
                }
                req.readyState = 'done'; req.result = db;
                req._dispatch('success');
            });
            return req;
        },
        deleteDatabase: function(name) {
            var req = new IDBOpenDBRequest(name);
            micro(function() { delete DBS[name]; req.readyState = 'done'; req.result = undefined; req._dispatch('success'); });
            return req;
        },
        databases: function() {
            var out = [];
            for (var k in DBS) if (Object.prototype.hasOwnProperty.call(DBS, k))
                out.push({ name: k, version: DBS[k].version });
            return Promise.resolve(out);
        },
        cmp: function(a, b) { return cmpKeys(a, b); }
    };

    globalThis.indexedDB = indexedDB;
    globalThis.IDBKeyRange = IDBKeyRange;
    globalThis.IDBRequest = IDBRequest;
    globalThis.IDBOpenDBRequest = IDBOpenDBRequest;
    globalThis.IDBDatabase = IDBDatabase;
    globalThis.IDBObjectStore = IDBObjectStore;
    globalThis.IDBTransaction = IDBTransaction;
    globalThis.IDBIndex = IDBIndex;
    globalThis.IDBCursor = IDBCursor;
    void 0;
})();
"#;
