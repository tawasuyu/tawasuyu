//! Drag&drop y portapapeles — `DataTransfer`, `DataTransferItem`,
//! `DataTransferItemList`, `DragEvent`, `ClipboardEvent`. Fase 7.109.
//!
//! Continúa la jerarquía de eventos (7.105-7.108). Las libs de drag&drop
//! (`react-dnd`, sortable.js) y de copiar/pegar leen `e.dataTransfer` /
//! `e.clipboardData` y llaman `dt.setData('text/plain', s)` / `dt.getData(...)`.
//! Sin el modelo `DataTransfer`, `new DragEvent(...)` deja `dataTransfer`
//! indefinido y el handler tira al primer `getData`.
//!
//! - `DataTransfer` — almacén de pares `{format → data}` (`setData`/`getData`/
//!   `clearData`/`types`), `dropEffect`/`effectAllowed`, `files` (FileList
//!   vacío), `items` (`DataTransferItemList`), `setDragImage()` no-op.
//! - `DataTransferItem` — una entrada (`kind`/`type`/`getAsString(cb)`/
//!   `getAsFile()→null`).
//! - `DataTransferItemList` — array-like con `add`/`remove`/`clear`.
//! - `DragEvent` extiende `MouseEvent` (7.105) — agrega `dataTransfer`.
//! - `ClipboardEvent` extiende `Event` — agrega `clipboardData`.
//!
//! Depende de que `UI_EVENTS_BOOTSTRAP` (7.105) corra antes (`DragEvent`
//! encadena a `MouseEvent.prototype`).
//!
//! **Limitaciones explícitas**:
//! 1. **`files` siempre vacío** — no hay drag de archivos reales del host
//!    headless; `items.add(file)` con un Blob/File se acepta pero `getAsFile`
//!    devuelve lo guardado verbatim sin validar.
//! 2. **Sólo constructores** — el chrome no despacha drag/drop/paste reales.
//! 3. **`setDragImage`** no-op (sin compositor que pinte el ghost).

pub(crate) const TRANSFER_EVENTS_BOOTSTRAP: &str = r#"
(function(){
  if (typeof globalThis.DataTransfer === 'function') return;
  if (typeof globalThis.Event !== 'function') return;

  // ---- DataTransferItem ----
  function DataTransferItem(kind, type, data) {
    this.kind = kind;
    this.type = type;
    this._data = data;
  }
  DataTransferItem.prototype.getAsString = function(cb) {
    if (typeof cb !== 'function') return;
    var s = (this.kind === 'string') ? String(this._data) : '';
    if (typeof globalThis.queueMicrotask === 'function') {
      globalThis.queueMicrotask(function(){ cb(s); });
    } else {
      globalThis.setTimeout(function(){ cb(s); }, 0);
    }
  };
  DataTransferItem.prototype.getAsFile = function() {
    return (this.kind === 'file') ? this._data : null;
  };

  // ---- DataTransferItemList ----
  function DataTransferItemList(store) {
    Object.defineProperty(this, '_store', { value: store, enumerable: false });
    this.length = 0;
  }
  function syncIndices(list) {
    var items = list._store.items;
    for (var i = 0; i < items.length; i++) list[i] = items[i];
    for (var j = items.length; j < list.length; j++) delete list[j];
    list.length = items.length;
  }
  DataTransferItemList.prototype.add = function(data, type) {
    var item;
    if (typeof data === 'string') {
      item = new DataTransferItem('string', String(type || ''), data);
      this._store.formats[String(type || '')] = data;
    } else {
      item = new DataTransferItem('file', (data && data.type) ? data.type : '', data);
    }
    this._store.items.push(item);
    syncIndices(this);
    return item;
  };
  DataTransferItemList.prototype.remove = function(i) {
    this._store.items.splice(i, 1);
    syncIndices(this);
  };
  DataTransferItemList.prototype.clear = function() {
    this._store.items.length = 0;
    this._store.formats = {};
    syncIndices(this);
  };
  DataTransferItemList.prototype.item = function(i) {
    return (i >= 0 && i < this._store.items.length) ? this._store.items[i] : null;
  };

  // ---- DataTransfer ----
  function DataTransfer() {
    var store = { formats: {}, items: [] };
    Object.defineProperty(this, '_store', { value: store, enumerable: false });
    this.dropEffect = 'none';
    this.effectAllowed = 'uninitialized';
    this.files = { length: 0, item: function(){ return null; } };
    this.items = new DataTransferItemList(store);
  }
  Object.defineProperty(DataTransfer.prototype, 'types', {
    get: function() { return Object.keys(this._store.formats); },
    enumerable: true
  });
  DataTransfer.prototype.setData = function(format, data) {
    format = String(format);
    var had = Object.prototype.hasOwnProperty.call(this._store.formats, format);
    this._store.formats[format] = String(data);
    if (!had) this.items.add(String(data), format);
    else {
      // actualizar el item string existente del mismo type
      for (var i = 0; i < this._store.items.length; i++) {
        var it = this._store.items[i];
        if (it.kind === 'string' && it.type === format) { it._data = String(data); break; }
      }
    }
  };
  DataTransfer.prototype.getData = function(format) {
    format = String(format);
    return Object.prototype.hasOwnProperty.call(this._store.formats, format)
      ? this._store.formats[format] : '';
  };
  DataTransfer.prototype.clearData = function(format) {
    if (format === undefined) { this.items.clear(); return; }
    format = String(format);
    delete this._store.formats[format];
    for (var i = this._store.items.length - 1; i >= 0; i--) {
      var it = this._store.items[i];
      if (it.kind === 'string' && it.type === format) this.items.remove(i);
    }
  };
  DataTransfer.prototype.setDragImage = function() {};

  // ---- DragEvent extends MouseEvent ----
  var DragBase = (typeof globalThis.MouseEvent === 'function') ? globalThis.MouseEvent : globalThis.Event;
  function DragEvent(type, init) {
    init = init || {};
    DragBase.call(this, type, init);
    this.dataTransfer = (init.dataTransfer !== undefined) ? init.dataTransfer : null;
  }
  DragEvent.prototype = Object.create(DragBase.prototype);
  DragEvent.prototype.constructor = DragEvent;

  // ---- ClipboardEvent extends Event ----
  function ClipboardEvent(type, init) {
    init = init || {};
    globalThis.Event.call(this, type, init);
    this.clipboardData = (init.clipboardData !== undefined) ? init.clipboardData : null;
  }
  ClipboardEvent.prototype = Object.create(globalThis.Event.prototype);
  ClipboardEvent.prototype.constructor = ClipboardEvent;

  globalThis.DataTransfer = DataTransfer;
  globalThis.DataTransferItem = DataTransferItem;
  globalThis.DataTransferItemList = DataTransferItemList;
  globalThis.DragEvent = DragEvent;
  globalThis.ClipboardEvent = ClipboardEvent;
})();
"#;
