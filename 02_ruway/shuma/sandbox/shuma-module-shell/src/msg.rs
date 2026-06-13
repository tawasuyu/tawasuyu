use super::*;


#[derive(Debug, Clone)]
pub enum Msg {
    /// Tecla recibida desde el chasis. Enter ejecuta, Tab completa,
    /// flechas y edición van al `LineState`.
    Key(KeyEvent),
    /// Click sobre el input box — re-foca y dirige el Enter a la "línea"
    /// (arrancar comandos nuevos), limpiando cualquier foco de stdin a un
    /// job vivo.
    FocusInput,
    /// Dirige el input al stdin del comando vivo del bloque dado (click o
    /// hover sobre su card). El Enter de la línea le manda el texto hasta
    /// que el usuario re-foca la línea u otro job, o el comando cierra.
    FocusJob(u64),
    /// Limpia el buffer de output — disparado por el shortcut `Clear`
    /// o el builtin `clear`.
    Clear,
    /// Drena eventos del run activo (si hay) y pinta líneas nuevas.
    /// Lo dispara el chasis a alta frecuencia (~100 ms).
    Tick,
    /// SIGTERM al run activo (Ctrl-C o shortcut `Cancel`).
    Cancel,
    /// Click en una decoración del output — el dispatch decide la
    /// acción (cd, xdg-open, pre-llenar el input, etc.).
    OpenDecoration(shuma_line::DecorationKind),
    /// Inserta `text` en la posición actual del cursor del input. La
    /// dispara el chasis cuando otro módulo (p. ej. `shuma-module-canvas`
    /// al clickear un nodo) quiere empujar una referencia `%pN`/`%cN`
    /// al REPL. Cierra los overlays de búsqueda y deja el cursor justo
    /// después del texto insertado.
    InsertAtCursor(String),
    /// Empuja un mensaje `Notice` al output sin abrir un bloque nuevo —
    /// para que el chasis (o cualquier consumidor) comunique fallas
    /// (podman, askpass, ...) en la vista del shell.
    PushNotice(String),
    /// Ajusta el zoom del texto del shell por un factor multiplicativo
    /// (e.g. 1.1 zoom in 10%, 1/1.1 zoom out). Ctrl+rueda lo dispara con
    /// pasos pequeños; Ctrl+= / Ctrl+- con pasos más grandes.
    ZoomBy(f32),
    /// Resetea el zoom a 1.0. Ctrl+0 lo dispara.
    ZoomReset,
    /// Mueve el scroll horizontal del shell por `dx` px (positivo = ver
    /// hacia la derecha del texto). Shift+rueda lo dispara. Cap a [0, ∞).
    ScrollHoriz(f32),
    /// Pega el clipboard al PTY del TUI activo — click derecho o botón
    /// del medio sobre el panel de vim (paste estilo terminal).
    VimPaste,
    /// Drag de selección sobre el card de vim. `dx`/`dy` = delta desde el
    /// evento anterior; `ax`/`ay` = posición del press (local al panel).
    VimDrag {
        end: bool,
        dx: f32,
        dy: f32,
        ax: f32,
        ay: f32,
    },
    /// Alterna plegado/desplegado de la card de un comando. La dispara el
    /// click en el header de la card (chevron + comando).
    ToggleBlock(u64),
    /// Alterna plegado/desplegado de una **sub-sección** dentro del bloque
    /// `block` (índice `idx` según `sections::detect_sections`). Click en
    /// el header de la sección lo dispara.
    ToggleSection { block: u64, idx: usize },
    /// Click en un header de columna de una sub-sección tipo tabla. Cicla:
    /// sin orden → asc(col) → desc(col) → sin orden.
    SortSectionColumn {
        block: u64,
        section: usize,
        col: usize,
    },
    /// Rueda del mouse sobre el panel de output. `delta` ya viene en px
    /// (positivo = rodar hacia arriba / ver historial). Ajusta `scroll_px`.
    Scroll(f32),
    /// Re-ejecuta `line` como un comando nuevo — la dispara el click en
    /// una etapa de pipe de una card SIN captura en vivo (fallback `sh -c`).
    RunLine(String),
    /// Alterna el desplegable de una etapa de pipe con captura en vivo
    /// (tee). La dispara el click en su chip; muestra/oculta las líneas
    /// intermedias ya capturadas sin re-ejecutar nada.
    ToggleStage { block: u64, stage: usize },
    /// Arma el reprocess: el stdout del bloque `block` alimentará el stdin
    /// del próximo comando. La dispara el chip ↻ de una card. Si ya estaba
    /// armado el mismo bloque, lo desarma (toggle).
    SetReprocess(u64),
    /// Ejecuta el grupo guardado de índice `idx` (0-based). La dispara el
    /// click en su card del panel de grupos (equivale a la tecla F{idx+1}).
    RunGroup(usize),
    /// A1 — acepta la coreografía emergente (identificada por su `signature`):
    /// la guarda como grupo ejecutable (F-key) con su nombre sugerido. La
    /// dispara el chip «guardar» sobre el input.
    AcceptChoreography(Vec<String>),
    /// A1 — descarta la oferta de coreografía (por `signature`): no se vuelve
    /// a ofrecer en la sesión. La dispara el chip «descartar».
    DismissChoreography(Vec<String>),
    /// A2 — acepta el alias para una línea larga repetida: lo agrega a la config
    /// viva y lo aprende al shumarc (`[aliases]`). La dispara el chip «aliasar».
    AcceptAlias(String),
    /// A2 — descarta la oferta de alias (por la línea): no se vuelve a ofrecer
    /// en la sesión. La dispara el chip «descartar» del alias.
    DismissAlias(String),
    /// A4 — acepta la corrección «¿quisiste decir…?» del bloque `block`: lleva
    /// la línea corregida al input (para revisarla y ejecutar con Enter) y
    /// limpia la oferta. La dispara el click en su notice.
    AcceptDidYouMean(u64),
    /// E2 — inserta la referencia `%cN` del bloque `block` al final del input
    /// (su stdout se materializa como fuente del pipe). La dispara el click en
    /// el tag `%cN` del header.
    InsertBlockRef(u64),
    /// Mouse sobre el cuerpo (IDE-text) de la card del bloque `block`:
    /// click posiciona el caret, drag extiende la selección. La dispara el
    /// `on_pointer` del `text-editor` del cuerpo.
    BodyPointer {
        block: u64,
        ev: llimphi_widget_text_editor::PointerEvent,
    },
    /// Copia al clipboard la selección viva del cuerpo del bloque `block`
    /// (click derecho sobre el cuerpo). No-op si no hay selección.
    CopyBody(u64),
    /// Copia al clipboard el **bloque entero** `block`: el comando (`$ …`)
    /// envuelto junto con su salida completa (stdout **y** stderr). La dispara
    /// el botón ⧉ del header de la card; no depende de que haya selección.
    CopyCommandBlock(u64),
    /// Doble-click sobre el cuerpo IDE-text del bloque `block`: selecciona
    /// la palabra bajo `(x, y)` (coords locales al nodo del editor, incluyen
    /// el gutter). La dispara el `on_double_tap_at` del cuerpo.
    BodyDoubleClick { block: u64, x: f32, y: f32 },
    /// Click derecho sobre el output: abre el menú contextual en `(x, y)`
    /// (coords del nodo raíz del shell = espacio de su view). Las acciones
    /// operan sobre el bloque seleccionado (o el más reciente).
    OpenBodyMenu { x: f32, y: f32 },
    /// Elegir un item del menú contextual del output (índice 0-based).
    BodyMenuPick(usize),
    /// Cerrar el menú contextual del output (scrim / Esc).
    BodyMenuDismiss,
    /// Click sobre el panel de un TUI bajo PTY (htop/less/btop/…). Si el
    /// programa habilitó mouse (`vt100::MouseProtocolMode != None`), encodea
    /// el click en xterm-mouse y lo escribe al stdin del PTY. `button` es 0
    /// (izquierdo), 1 (medio), 2 (derecho). `lx`/`ly` son coords relativas
    /// al rect del panel; `rect_w`/`rect_h` el tamaño del rect (para
    /// convertir a celdas).
    TuiMouseClick {
        button: u8,
        lx: f32,
        ly: f32,
        rect_w: f32,
        rect_h: f32,
    },
    /// Rueda sobre el panel TUI. `dy` positivo = arriba (botón 4); negativo
    /// = abajo (botón 5). Se emite un evento de mouse por cada "tick" de
    /// rueda lógica. Las coords se usan para reportar dónde estaba el
    /// cursor (algunos TUIs lo respetan).
    TuiMouseWheel {
        dy: f32,
        lx: f32,
        ly: f32,
        rect_w: f32,
        rect_h: f32,
    },
    /// Drag del mouse sobre el cuerpo de output en modo **superficie**
    /// (`SHUMA_TERMINAL_SURFACE=1`). El primer Move arranca/colapsa la
    /// selección al `(lx0, ly0)`; los siguientes la extienden; el End la
    /// deja fijada para que el usuario copie. `dx`/`dy` son deltas desde el
    /// evento previo (el `update` los acumula sobre `(ax, ay)`).
    SurfSelectDrag {
        phase: llimphi_ui::DragPhase,
        dx: f32,
        dy: f32,
        ax: f32,
        ay: f32,
    },
    /// Limpia la selección viva del cuerpo de output (lo dispara una tecla,
    /// un click en blanco, etc.). No-op si ya está vacía.
    SurfClearSelection,
    /// Copia al clipboard el texto de la selección viva del cuerpo de
    /// output. No-op si no hay selección. Reusa el clipboard global del
    /// proceso (vía `arboard`).
    SurfCopySelection,
    /// Doble-click sobre el cuerpo de output en modo superficie: selecciona
    /// la palabra bajo el punto (paridad con terminales clásicas). El
    /// `update` resuelve `(lx, ly)` a `Point` con `point_at_geo`, computa
    /// los boundaries de palabra en el texto de la línea y arma una
    /// `SelectionRange` sobre esa palabra.
    SurfDoubleClick {
        lx: f32,
        ly: f32,
        rect_w: f32,
        rect_h: f32,
    },
    /// Right-click sobre el cuerpo de output en modo superficie: abre el
    /// menú contextual en `(x, y)` (coords del nodo raíz del shell). Las
    /// acciones operan sobre el scrollback entero (no por-bloque como el
    /// `BodyMenu` del legacy) — Copiar selección, Copiar todo, Seleccionar
    /// todo.
    SurfOpenMenu { x: f32, y: f32 },
    /// Elegir un item del menú contextual del surface (0-based).
    SurfMenuPick(usize),
    /// Cerrar el menú contextual del surface (scrim / Esc).
    SurfMenuDismiss,
    /// Abre la barra de búsqueda (Ctrl+F). Si ya estaba abierta, re-foca el
    /// input vacío (paridad con browsers/editores). Si no hay layout
    /// publicado todavía, abre igual — el primer keystroke recomputará.
    FindOpen,
    /// Cierra la barra de búsqueda (Esc). Limpia `find` y la selección
    /// derivada de un match. No toca `surf_selection` si vino de un drag
    /// del mouse y no de un match (la heurística: si `find` existía y
    /// tenía un `current`, era nuestro highlight; lo limpiamos).
    FindClose,
    /// Agrega un char a la query de búsqueda y re-busca.
    FindChar(char),
    /// Borra el último char de la query y re-busca.
    FindBackspace,
    /// Avanza al siguiente match (Enter / F3 / botón).
    FindNext,
    /// Retrocede al match previo (Shift+Enter / Shift+F3 / botón).
    FindPrev,
    /// Togglea case-insensitive (botón `Aa` o atajo). Re-busca con la
    /// nueva política.
    FindToggleCase,
    /// E5 — resultado de una invocación al LLM (`:?`/`:explica`/`:resume`).
    /// Lo dispatcha el host (chasis) tras correr `pluma-llm` en un thread;
    /// el módulo sólo expresó la intención (`State::llm_request`). `kind`
    /// decide el destino: `Command` → al input (revisar y Enter, NUNCA
    /// auto-ejecuta), `Text` → al output del bloque.
    LlmResult {
        kind: LlmKind,
        ok: bool,
        text: String,
    },
}
