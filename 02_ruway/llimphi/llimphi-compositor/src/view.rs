use super::*;

impl<Msg> View<Msg> {
    pub fn new(style: Style) -> Self {
        Self {
            style,
            fill: None,
            hover_fill: None,
            radius: 0.0,
            corner_radii: None,
            shadow: None,
            fill_gradient: None,
            border: None,
            text: None,
            image: None,
            image_fit: None,
            painter: None,
            gpu_painter: None,
            on_pointer_enter: None,
            on_pointer_leave: None,
            on_pointer_move_at: None,
            on_click: None,
            on_click_at: None,
            on_right_click: None,
            on_right_click_at: None,
            on_middle_click: None,
            drag: None,
            drag_at: None,
            drag_velocity: None,
            drag_payload: None,
            on_drop: None,
            drop_hover_fill: None,
            clip: false,
            clip_inset: None,
            clip_ellipse: None,
            on_scroll: None,
            on_scale: None,
            on_rotate: None,
            on_double_tap: None,
            on_double_tap_at: None,
            on_long_press: None,
            on_long_press_at: None,
            focusable: None,
            text_select_key: None,
            alpha: None,
            anim: None,
            animated_size: None,
            semantics: None,
            hero: None,
            transform: None,
            transform_rel: None,
            tooltip: None,
            cursor: None,
            ripple: None,
            layout_builder: None,
            backdrop_blur: None,
            children: Vec::new(),
        }
    }

    /// Aplica un **backdrop blur** Gaussiano al contenido pintado **debajo**
    /// de este nodo, restringido al rect del nodo (CSS `backdrop-filter:
    /// blur(N)` / Flutter `BackdropFilter`). El runtime descompone el árbol
    /// en "fondo + subárbol del nodo", renderiza el fondo a la intermediate,
    /// borronea el rect con un Gauss separable, y compone el subárbol del
    /// nodo sobre el backdrop borroso vía un buffer secundario. Útil para
    /// chrome translúcido: sidebars/topbars con "vidrio esmerilado".
    ///
    /// `sigma` (pixels) controla el ancho del kernel — `4.0` "frosted glass"
    /// suave; `8.0`–`16.0` un blur fuerte; >`20` se ve apagado. v1 capa el
    /// radius efectivo a 32 pixels (sigma > 10 empieza a clipear cola).
    ///
    /// **Limitación v1**: sólo nodos top-level (children directos del root o
    /// de un agrupador sin `clip`/`alpha`) renderizan correctamente — un
    /// nodo dentro de una capa clippeada se pinta SIN clip en su pase, por
    /// el reset de layer-stack al cambiar de scene. Documentado en
    /// `PARIDAD-FLUTTER.md` Bloque 11.
    pub fn backdrop_blur(mut self, sigma: f32) -> Self {
        self.backdrop_blur = Some(sigma.max(0.0));
        self
    }

    /// Construye los hijos de este nodo **de forma diferida**, en función del
    /// tamaño del slot que el layout le asigne (Flutter `LayoutBuilder`). El
    /// runtime resuelve primero el rect del nodo (una pasada de layout con este
    /// nodo como hoja, sized por su `Style`/contexto flex) y recién entonces
    /// invoca `builder(Constraints)` para producir el subárbol — habilitando
    /// paneles responsive cuyo punto de quiebre depende del **espacio local**,
    /// no de la ventana (para eso alcanza `on_resize` + el Model).
    ///
    /// El `Style` de este nodo define su tamaño (debe quedar acotado por el
    /// contexto: `flex_grow`, `size` definido o `percent` — no intrínseco a los
    /// hijos, que aún no existen). Cualquier `children` estático que se haya
    /// seteado se ignora: el builder es la fuente de los hijos.
    ///
    /// **Límite v1**: sin anidamiento — un `layout_builder` dentro del subárbol
    /// que produce otro `layout_builder` no se resuelve (queda como hoja).
    pub fn layout_builder<F>(mut self, builder: F) -> Self
    where
        F: Fn(Constraints) -> View<Msg> + Send + Sync + 'static,
    {
        self.layout_builder = Some(Arc::new(builder));
        self
    }

    /// Marca este nodo para emitir un **ripple/InkWell** (la salpicadura de tap
    /// de Material) al recibir un press: un círculo que se expande desde el
    /// punto presionado y se desvanece, recortado al contorno del nodo. `key`
    /// debe ser **estable** entre rebuilds del `View` (índice/hash del item),
    /// igual que la key de [`Self::animated`]. `color` es el tinte de la onda —
    /// usá un color semitransparente (blanco a alpha ~0.25 sobre superficies
    /// oscuras, negro a alpha ~0.12 sobre claras); su alpha se atenúa con el
    /// fade. Es **aditivo**: convive con `on_click`/`drag` sin pisarlos. Duración
    /// por defecto 450 ms; para otra usar [`Self::ripple_styled`].
    pub fn ripple(self, key: u64, color: Color) -> Self {
        self.ripple_styled(key, color, std::time::Duration::from_millis(450))
    }

    /// Como [`Self::ripple`] pero con la duración explícita de la salpicadura.
    pub fn ripple_styled(
        mut self,
        key: u64,
        color: Color,
        duration: std::time::Duration,
    ) -> Self {
        self.ripple = Some(Ripple { key, color, duration });
        self
    }

    /// Fija la forma del puntero del mouse mientras el cursor está sobre este
    /// nodo (o un descendiente que no declare la suya — se hereda del ancestro
    /// más cercano que la tenga). El runtime la resuelve en el hit-test de hover
    /// y la aplica a la ventana. Ejemplos: `.cursor(Cursor::Text)` en un input,
    /// `.cursor(Cursor::ColResize)` en un divisor de splitter,
    /// `.cursor(Cursor::Pointer)` en un botón.
    pub fn cursor(mut self, cursor: Cursor) -> Self {
        self.cursor = Some(cursor);
        self
    }

    /// Asocia un texto de **tooltip** a este nodo. Llimphi sólo lo transporta
    /// hasta el [`MountedNode`](crate::MountedNode); el consumidor decide cómo
    /// mostrarlo (un overlay del runtime, una surface popup del cliente) tras
    /// localizar el nodo bajo el cursor con el hit-test de hover.
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    /// Declara la **semántica accesible** completa del nodo de una vez. Usar
    /// cuando ya tenés un [`SemanticsSpec`] armado (p. ej. construido por un
    /// widget); para los casos puntuales preferí los atajos
    /// [`Self::role`]/[`Self::aria_label`]/etc.
    pub fn semantics(mut self, spec: SemanticsSpec) -> Self {
        self.semantics = Some(spec);
        self
    }

    /// Fija el **rol** semántico del nodo. Si ya había semántica declarada,
    /// preserva label/value/flags y sólo sobreescribe el rol; si no, crea una
    /// `SemanticsSpec` con sólo el rol.
    pub fn role(mut self, role: Role) -> Self {
        self.semantics = Some(match self.semantics.take() {
            Some(mut s) => {
                s.role = Some(role);
                s
            }
            None => SemanticsSpec::role(role),
        });
        self
    }

    /// Fija el **label accesible** ("nombre" que el lector enuncia). Hace falta
    /// cuando el contenido visible del nodo no alcanza (p. ej. un botón con
    /// sólo un ícono). Preserva el resto de la `SemanticsSpec` si existía.
    pub fn aria_label(mut self, label: impl Into<std::sync::Arc<str>>) -> Self {
        let mut s = self.semantics.take().unwrap_or_default();
        s.label = Some(label.into());
        self.semantics = Some(s);
        self
    }

    /// Fija la **descripción** (contexto adicional que el lector enuncia tras
    /// el label, típicamente con un atajo). Para info que ayuda pero no es el
    /// nombre principal — no abusar (los lectores perciben ruido).
    pub fn aria_description(mut self, desc: impl Into<std::sync::Arc<str>>) -> Self {
        let mut s = self.semantics.take().unwrap_or_default();
        s.description = Some(desc.into());
        self.semantics = Some(s);
        self
    }

    /// Fija el **valor** (texto del input, valor del slider/spinner). Lo que
    /// el lector lee después del label: "Volumen, 70".
    pub fn aria_value(mut self, value: impl Into<std::sync::Arc<str>>) -> Self {
        let mut s = self.semantics.take().unwrap_or_default();
        s.value = Some(value.into());
        self.semantics = Some(s);
        self
    }

    /// Estado `checked` (checkbox/radio).
    pub fn aria_checked(mut self, v: bool) -> Self {
        let mut s = self.semantics.take().unwrap_or_default();
        s.flags.checked = Some(v);
        self.semantics = Some(s);
        self
    }

    /// Estado `pressed` (toggle button).
    pub fn aria_pressed(mut self, v: bool) -> Self {
        let mut s = self.semantics.take().unwrap_or_default();
        s.flags.pressed = Some(v);
        self.semantics = Some(s);
        self
    }

    /// Estado `expanded` (acordeón, menú abierto, tree row expandida).
    pub fn aria_expanded(mut self, v: bool) -> Self {
        let mut s = self.semantics.take().unwrap_or_default();
        s.flags.expanded = Some(v);
        self.semantics = Some(s);
        self
    }

    /// Estado `disabled` — el control no responde a input.
    pub fn aria_disabled(mut self, v: bool) -> Self {
        let mut s = self.semantics.take().unwrap_or_default();
        s.flags.disabled = Some(v);
        self.semantics = Some(s);
        self
    }

    /// Estado `readonly` — el control es visible/seleccionable pero no editable.
    pub fn aria_readonly(mut self, v: bool) -> Self {
        let mut s = self.semantics.take().unwrap_or_default();
        s.flags.readonly = Some(v);
        self.semantics = Some(s);
        self
    }

    /// Estado `required` (campo de formulario obligatorio).
    pub fn aria_required(mut self, v: bool) -> Self {
        let mut s = self.semantics.take().unwrap_or_default();
        s.flags.required = Some(v);
        self.semantics = Some(s);
        self
    }

    /// Registra un handler de rueda local: si el cursor está sobre este
    /// nodo cuando la rueda gira, el runtime lo invoca con el delta
    /// `(dx, dy)` en líneas lógicas ANTES de caer al `App::on_wheel`
    /// global. Devolver `Some(Msg)` consume el evento. Es la base de las
    /// áreas de scroll autocontenidas (`llimphi-widget-scroll`).
    pub fn on_scroll<F>(mut self, handler: F) -> Self
    where
        F: Fn(f32, f32) -> Option<Msg> + Send + Sync + 'static,
    {
        self.on_scroll = Some(Arc::new(handler));
        self
    }

    /// Registra un handler de **pinch-to-zoom** (gesto de escala). El runtime
    /// lo invoca cuando el cursor está sobre este nodo y el usuario hace un
    /// gesto de escala: **Ctrl + rueda** en cualquier desktop (camino
    /// universal) o un pinch de trackpad en macOS. El handler recibe
    /// `(phase, factor, focal_x, focal_y)` — ver [`ScaleFn`]: `factor` es el
    /// cambio multiplicativo incremental (`>1` agranda, `<1` achica) y
    /// `(focal_x, focal_y)` es el punto bajo el cursor relativo al rect del
    /// nodo, para zoomear "hacia el cursor". El típico patrón de canvas:
    /// `Msg::Zoom { factor, fx, fy }` que multiplica la escala del viewport y
    /// reajusta el pan para mantener el punto focal fijo. Devolver `Some(Msg)`
    /// consume el gesto (no cae al scroll/`on_wheel`).
    pub fn on_scale<F>(mut self, handler: F) -> Self
    where
        F: Fn(GesturePhase, f32, f32, f32) -> Option<Msg> + Send + Sync + 'static,
    {
        self.on_scale = Some(Arc::new(handler));
        self
    }

    /// Registra un handler de **rotación con dos dedos** (gesto de trackpad).
    /// El runtime lo invoca cuando el cursor está sobre este nodo y el usuario
    /// rota dos dedos en el trackpad (winit emite `RotationGesture` **sólo en
    /// macOS**). El handler recibe `(phase, delta_radianes, focal_x, focal_y)`
    /// — ver [`RotateFn`]: `delta_radianes` es el incremento angular (positivo
    /// = horario) y `(focal_x, focal_y)` el punto bajo el cursor relativo al
    /// rect del nodo, para rotar "alrededor del cursor". Patrón típico de
    /// canvas/imagen: `Msg::Rotate { delta, fx, fy }` que acumula el ángulo del
    /// viewport. Devolver `Some(Msg)` consume el gesto.
    pub fn on_rotate<F>(mut self, handler: F) -> Self
    where
        F: Fn(GesturePhase, f32, f32, f32) -> Option<Msg> + Send + Sync + 'static,
    {
        self.on_rotate = Some(Arc::new(handler));
        self
    }

    /// Emite `msg` en **doble-tap** (dos clicks izquierdos rápidos y cercanos
    /// sobre este nodo). Aditivo respecto de `on_click`. Ver
    /// [`Self::on_double_tap`](#structfield.on_double_tap) (campo) para la
    /// semántica completa; para la posición del tap usar
    /// [`Self::on_double_tap_at`].
    pub fn on_double_tap(mut self, msg: Msg) -> Self {
        self.on_double_tap = Some(msg);
        self
    }

    /// Como [`Self::on_double_tap`] pero el handler recibe la posición del
    /// segundo tap relativa al rect del nodo `(lx, ly, w, h)` — para
    /// zoom-to-point o seleccionar la entidad bajo el cursor. Gana sobre
    /// `on_double_tap` si ambos están.
    pub fn on_double_tap_at<F>(mut self, handler: F) -> Self
    where
        F: Fn(f32, f32, f32, f32) -> Option<Msg> + Send + Sync + 'static,
    {
        self.on_double_tap_at = Some(Arc::new(handler));
        self
    }

    /// Emite `msg` en **long-press** (mantener el botón ~500 ms sin moverse).
    /// El runtime lo cancela si el cursor se aleja (pasó a drag) o se suelta
    /// antes. Aditivo respecto de `on_click`/`drag`. Ver
    /// [`Self::on_long_press`](#structfield.on_long_press) (campo); para la
    /// posición usar [`Self::on_long_press_at`].
    pub fn on_long_press(mut self, msg: Msg) -> Self {
        self.on_long_press = Some(msg);
        self
    }

    /// Como [`Self::on_long_press`] pero el handler recibe la posición del
    /// press relativa al rect del nodo `(lx, ly, w, h)` — para abrir un menú
    /// contextual en el punto. Gana sobre `on_long_press` si ambos están.
    pub fn on_long_press_at<F>(mut self, handler: F) -> Self
    where
        F: Fn(f32, f32, f32, f32) -> Option<Msg> + Send + Sync + 'static,
    {
        self.on_long_press_at = Some(Arc::new(handler));
        self
    }

    /// Marca este nodo como enfocable con el id opaco `id`. El runtime lo
    /// incluye en el orden de Tab (pre-orden del árbol) y le da foco al
    /// clickearlo; cada cambio de foco se notifica vía `App::on_focus`.
    /// El caller pinta el focus-ring comparando el id contra el foco que
    /// guardó en su `Model`.
    pub fn focusable(mut self, id: u64) -> Self {
        self.focusable = Some(id);
        self
    }

    /// Marca este nodo de **texto** como seleccionable con el mouse fuera del
    /// editor: arrastrar sobre él resalta el rango y Ctrl/Cmd+C lo copia al
    /// portapapeles. `key` debe ser **estable** entre rebuilds del `View`
    /// (índice, hash del id) — la selección vive en el runtime anclada a esa
    /// key, no al `NodeId` (que cambia cada frame). Pensá en labels, párrafos,
    /// celdas de tabla, salidas de consola: cualquier texto que el usuario
    /// querría copiar sin un editor. Sólo aplica a texto **uniforme** (el de
    /// `.text(...)`/`.text_aligned(...)`); en nodos con `runs`/`spans` no tiene
    /// efecto (esos son del editor / RichText). Componer con el texto:
    /// `View::new(style).text_aligned(s, 14.0, col, al).selectable(key)`.
    pub fn selectable(mut self, key: u64) -> Self {
        self.text_select_key = Some(key);
        self
    }

    /// Marca este nodo como **hero shared-element** con la `key` indicada.
    /// Cuando la misma `key` aparece en un rect distinto en el frame siguiente
    /// (entre rutas, paneles, layouts), el runtime interpola `transform` para
    /// "volar" del rect anterior al actual durante `duration`. La `key` debe
    /// ser estable y única dentro del frame; idéntica semántica a la `key`
    /// de [`Self::animated`]. Para easing distinto al ease-out cúbico default,
    /// ver [`Self::hero_curve`].
    pub fn hero(mut self, key: u64, duration: std::time::Duration) -> Self {
        self.hero = Some(Hero {
            key,
            duration,
            easing: ease_out_cubic,
        });
        self
    }

    /// Como [`Self::hero`] pero con easing explícito.
    pub fn hero_curve(
        mut self,
        key: u64,
        duration: std::time::Duration,
        easing: fn(f32) -> f32,
    ) -> Self {
        self.hero = Some(Hero { key, duration, easing });
        self
    }

    /// Aplica una transformación afín 2D a este nodo y todo su subtree,
    /// **alrededor del centro de su rect** (CSS `transform-origin: 50%
    /// 50%`). El centro se resuelve en `paint` contra el layout computado;
    /// el caller sólo provee el afín "local" (producto de sus
    /// `rotate`/`scale`/`translate`). Nodos anidados componen en el
    /// espacio ya transformado del padre. Pensado para `transform` y
    /// `@keyframes` CSS de puriy. `Affine::IDENTITY` equivale a no setear.
    pub fn transform(mut self, xf: Affine) -> Self {
        self.transform = Some(xf);
        self
    }

    /// Traslación relativa al tamaño del propio nodo: `(fx, fy)` desplaza
    /// `(fx · w, fy · h)` px, resueltos contra el rect computado en `paint`.
    /// Es el `translate(<%>)` de CSS que no cabe en un `Affine` fijo (p. ej.
    /// el centrado `translate(-50%, -50%)` ⇒ `transform_rel((-0.5, -0.5))`).
    /// Compone con `transform` (si está) como factor más externo. Ver
    /// [`View::transform`]. `(0.0, 0.0)` equivale a no setear.
    pub fn transform_rel(mut self, frac: (f64, f64)) -> Self {
        self.transform_rel = Some(frac);
        self
    }

    pub fn fill(mut self, color: Color) -> Self {
        self.fill = Some(color);
        self
    }

    /// Opacidad uniforme aplicada a este nodo y todos sus descendientes
    /// vía `scene.push_layer(Mix::Normal, a, …)`. Pensado para fade-in/out
    /// de overlays, toasts y modales sin tener que tunear el alpha de
    /// cada color del subtree. Valores fuera de `[0.0, 1.0]` se clampean.
    /// Hace que el subtree se componga en una capa intermedia — usar sólo
    /// cuando sea necesario (no es gratuito).
    pub fn alpha(mut self, a: f32) -> Self {
        self.alpha = Some(a.clamp(0.0, 1.0));
        self
    }

    /// Anima de forma **implícita** las props de paint de este nodo
    /// (hoy `fill` y `radius`): cuando su valor cambia entre frames, el
    /// runtime interpola en `duration` con ease-out cúbico en vez de saltar
    /// (estilo Flutter `AnimatedContainer`). `key` debe ser **estable** entre
    /// rebuilds del `View` (índice de item, hash de id) — es lo que enlaza
    /// "el mismo nodo" entre frames; dos nodos distintos no deben compartir
    /// key. La primera aparición no anima; sólo los cambios posteriores. Para
    /// otra curva, [`Self::animated_curve`].
    pub fn animated(mut self, key: u64, duration: std::time::Duration) -> Self {
        self.anim = Some(Anim {
            key,
            duration,
            easing: ease_out_cubic,
            enter: false,
            exit: false,
            enter_from_xf: None,
            switch: None,
        });
        self
    }

    /// Anima de forma **implícita** el **tamaño** de este nodo (Flutter
    /// `AnimatedSize` / Compose `animateContentSize()`). Cuando
    /// `style.size` cambia entre frames, el runtime interpola en
    /// `duration` con ease-out cúbico en vez de saltar — siblings y
    /// hijos reflowean suave porque el reconciler parcha `style.size`
    /// **antes** del layout. `key` debe ser estable entre rebuilds.
    /// Para otra curva, [`Self::animated_size_curve`]. Bloque 15.
    ///
    /// **Límite v1**: ambos `style.size.width` y `style.size.height`
    /// tienen que ser `Dimension::Length(_)`. Si una es `Percent`/`Auto`,
    /// el nodo se monta tal cual sin animación (no hay valor en píxeles
    /// estable para interpolar). El caller que necesite animar un nodo
    /// flex puede envolver el contenido en un wrap con `length(...)`
    /// fijo y mover el flex al padre.
    pub fn animated_size(mut self, key: u64, duration: std::time::Duration) -> Self {
        self.animated_size = Some(SizeAnim {
            key,
            duration,
            easing: ease_out_cubic,
        });
        self
    }

    /// Como [`Self::animated_size`] pero con curva de easing custom.
    pub fn animated_size_curve(
        mut self,
        key: u64,
        duration: std::time::Duration,
        easing: fn(f32) -> f32,
    ) -> Self {
        self.animated_size = Some(SizeAnim { key, duration, easing });
        self
    }

    /// Como [`Self::animated`] pero además **anima la entrada**: la primera vez
    /// que esta `key` aparece, su opacidad sube de 0 a su valor (`alpha` o 1.0)
    /// en `duration` — fade-in estilo `AnimatedSwitcher`/`AnimatedVisibility`.
    /// Útil para toasts, items de lista que aparecen, paneles que se montan,
    /// resultados que entran. Como toda animación implícita, depende de una
    /// `key` estable; reutilizar la key de un nodo que ya estaba NO refadea
    /// (sólo la primera aparición anima). Para animar también la salida, ver
    /// [`Self::animated_inout`].
    pub fn animated_enter(mut self, key: u64, duration: std::time::Duration) -> Self {
        self.anim = Some(Anim {
            key,
            duration,
            easing: ease_out_cubic,
            enter: true,
            exit: false,
            enter_from_xf: None,
            switch: None,
        });
        self
    }

    /// Como [`Self::animated_enter`] pero además arranca la entrada desde una
    /// **transformación afín** específica hacia la del nodo (o la identidad si
    /// no setea `.transform`). Habilita scale-in / slide-in / rotate-in
    /// implícitos: el caller declara la pose inicial, el runtime interpola.
    /// Ejemplos:
    ///   - `Affine::scale(0.6)` → "pop" (FAB de Material, modales).
    ///   - `Affine::translate((0.0, 60.0))` → slide-in vertical (snackbars).
    ///   - `Affine::translate((-w, 0.0))` → slide-in lateral (drawers).
    /// Combina con el fade-in de entrada (`alpha 0 → opaque`). Sin animación
    /// de salida; para entrada+salida con pose, ver
    /// [`Self::animated_inout_from`].
    pub fn animated_enter_from(
        mut self,
        key: u64,
        duration: std::time::Duration,
        from_xf: Affine,
    ) -> Self {
        self.anim = Some(Anim {
            key,
            duration,
            easing: ease_out_cubic,
            enter: true,
            exit: false,
            enter_from_xf: Some(from_xf),
            switch: None,
        });
        self
    }

    /// Atajo Material: scale-in desde 0.6 (entrada "pop" del FAB). Combina con
    /// fade-in. Equivalente a `.animated_enter_from(key, dur, Affine::scale(0.6))`.
    pub fn animated_pop_in(self, key: u64, duration: std::time::Duration) -> Self {
        self.animated_enter_from(key, duration, Affine::scale(0.6))
    }

    /// **Anima la salida** (fade-out): cuando esta `key` desaparece del árbol,
    /// el runtime retiene la última subescena que pintó y la reproduce con
    /// opacidad decreciente durante `duration` — estilo `AnimatedSwitcher` /
    /// `AnimatedVisibility` al ocultarse. No anima la entrada (para ambas, ver
    /// [`Self::animated_inout`]). Tiene coste por frame mientras el nodo vive
    /// (captura su subárbol); usar con moderación (toasts, modales, paneles).
    pub fn animated_exit(mut self, key: u64, duration: std::time::Duration) -> Self {
        self.anim = Some(Anim {
            key,
            duration,
            easing: ease_out_cubic,
            enter: false,
            exit: true,
            enter_from_xf: None,
            switch: None,
        });
        self
    }

    /// Anima **entrada y salida**: fade-in en la primera aparición y fade-out al
    /// desmontarse, ambos en `duration`. La pieza completa de "animación de
    /// contenido" para un nodo que aparece y desaparece (un toast, un panel que
    /// se abre y cierra, un resultado que entra y se va).
    pub fn animated_inout(mut self, key: u64, duration: std::time::Duration) -> Self {
        self.anim = Some(Anim {
            key,
            duration,
            easing: ease_out_cubic,
            enter: true,
            exit: true,
            enter_from_xf: None,
            switch: None,
        });
        self
    }

    /// Como [`Self::animated_inout`] pero arranca la entrada desde la
    /// transformación afín `from_xf` (igual semántica que
    /// [`Self::animated_enter_from`]). La salida sigue siendo el fade-out
    /// estándar (la subescena retenida no transforma).
    pub fn animated_inout_from(
        mut self,
        key: u64,
        duration: std::time::Duration,
        from_xf: Affine,
    ) -> Self {
        self.anim = Some(Anim {
            key,
            duration,
            easing: ease_out_cubic,
            enter: true,
            exit: true,
            enter_from_xf: Some(from_xf),
            switch: None,
        });
        self
    }

    /// Como [`Self::animated`] pero con easing explícito (p. ej.
    /// `llimphi_theme::motion::ease_in_out_cubic`).
    pub fn animated_curve(
        mut self,
        key: u64,
        duration: std::time::Duration,
        easing: fn(f32) -> f32,
    ) -> Self {
        self.anim = Some(Anim {
            key,
            duration,
            easing,
            enter: false,
            exit: false,
            enter_from_xf: None,
            switch: None,
        });
        self
    }

    /// Cross-fade real entre **variantes de contenido** bajo la misma `key`
    /// (Flutter `AnimatedSwitcher`). `variant` identifica el contenido actual
    /// (índice de pestaña, hash del estado, discriminante de un enum…). Cuando
    /// `variant` cambia entre frames, el runtime desvanece la subescena vieja
    /// (fade-out, retenida del frame previo) mientras hace fade-in del subárbol
    /// nuevo, en el mismo rect — la transición real entre dos identidades, en
    /// vez de combinar `animated_enter`+`animated_exit` de dos keys distintas.
    ///
    /// Envolvé el contenido conmutable en un nodo con esta marca; sus hijos son
    /// el contenido. La primera aparición no cruza (sólo fija la variante).
    /// Igual que `exit`, captura el subárbol por frame — usar en pocos nodos
    /// (un panel central, un visor que cambia de documento), no por fila.
    pub fn animated_switch(
        mut self,
        key: u64,
        variant: u64,
        duration: std::time::Duration,
    ) -> Self {
        self.anim = Some(Anim {
            key,
            duration,
            easing: ease_out_cubic,
            enter: false,
            exit: false,
            enter_from_xf: None,
            switch: Some(variant),
        });
        self
    }

    /// Color a usar cuando el cursor está sobre este nodo. Habilita
    /// el hit-test de hover sobre el nodo.
    pub fn hover_fill(mut self, color: Color) -> Self {
        self.hover_fill = Some(color);
        self
    }

    /// Marca este nodo como draggable. Mientras el usuario sostenga el
    /// botón izquierdo sobre él, el runtime llama `handler(Move, dx, dy)`
    /// por cada `CursorMoved` (dx/dy = delta desde el evento anterior) y
    /// `handler(End, 0, 0)` al soltar. Sobreescribe `on_click` para este
    /// nodo: un nodo es draggable **o** clickable.
    pub fn draggable<F>(mut self, handler: F) -> Self
    where
        F: Fn(DragPhase, f32, f32) -> Option<Msg> + Send + Sync + 'static,
    {
        self.drag = Some(Arc::new(handler));
        self
    }

    /// Como `draggable`, pero el handler también recibe la posición
    /// inicial del press relativa al rect del nodo `(initial_lx,
    /// initial_ly)`. Útil cuando el caller necesita resolver qué
    /// entidad bajo el cursor inició el drag (Conceptos, lemmings,
    /// nodos de un grafo, etc.). Gana sobre `draggable` si ambos están.
    pub fn draggable_at<F>(mut self, handler: F) -> Self
    where
        F: Fn(DragPhase, f32, f32, f32, f32) -> Option<Msg> + Send + Sync + 'static,
    {
        self.drag_at = Some(Arc::new(handler));
        self
    }

    /// Como [`Self::draggable`] pero el handler recibe además la **velocidad
    /// del drag al soltarlo** (`vx`, `vy` en px/s) en la fase
    /// `DragPhase::End` — `Fn(DragPhase, dx, dy, vx, vy) -> Option<Msg>`. El
    /// runtime mide el desplazamiento sobre los últimos ~100 ms y lo divide
    /// por el tiempo transcurrido. Durante `DragPhase::Move`, `vx == vy == 0`
    /// (la velocidad sólo se calcula al final). **Gana sobre `draggable` y
    /// `draggable_at`** si conviven en el mismo nodo — un nodo elige un
    /// único sabor de drag. Habilita **fling-desde-drag**: el caller emite
    /// `Msg::Fling { vx, vy }` en End y arranca un ticker que decae la
    /// velocidad con [`fling_step`] hasta asentar.
    pub fn draggable_velocity<F>(mut self, handler: F) -> Self
    where
        F: Fn(DragPhase, f32, f32, f32, f32) -> Option<Msg> + Send + Sync + 'static,
    {
        self.drag_velocity = Some(Arc::new(handler));
        self
    }

    /// Declara el payload `u64` que viaja con el drag de este nodo. Los
    /// drop targets bajo cursor al soltar reciben este valor en su
    /// `on_drop`. Sin payload, los drop targets no reaccionan (útil para
    /// drags de "resize/scroll" que no representan transferencia).
    pub fn drag_payload(mut self, payload: u64) -> Self {
        self.drag_payload = Some(payload);
        self
    }

    /// Marca este nodo como drop target. El runtime invoca `handler(payload)`
    /// cuando un drag termina sobre el rect de este nodo y el origen del
    /// drag declaró un payload. Si devuelve `Some(Msg)`, se dispatchea al
    /// `update` antes del `DragPhase::End` del origen.
    pub fn on_drop<F>(mut self, handler: F) -> Self
    where
        F: Fn(u64) -> Option<Msg> + Send + Sync + 'static,
    {
        self.on_drop = Some(Arc::new(handler));
        self
    }

    /// Color de relleno cuando un drag activo está hovereando este drop
    /// target. Análogo a `hover_fill` pero solo aplica mientras dura un
    /// drag. Útil para resaltar el destino válido.
    pub fn drop_hover_fill(mut self, color: Color) -> Self {
        self.drop_hover_fill = Some(color);
        self
    }

    pub fn radius(mut self, r: f64) -> Self {
        self.radius = r;
        self
    }

    /// Radio **por esquina** (top-left, top-right, bottom-right, bottom-left,
    /// en sentido horario desde arriba-izquierda) — CSS `border-radius` con
    /// cuatro valores. Sobreescribe a [`Self::radius`] mientras esté presente.
    /// Para cards con sólo las esquinas de arriba redondeadas, pestañas,
    /// bocadillos de chat asimétricos, etc. El **borde** respeta las cuatro
    /// esquinas; la **sombra** sigue usando el `radius` escalar (el blur
    /// nativo de vello no acepta radios por esquina).
    pub fn radius_corners(mut self, tl: f64, tr: f64, br: f64, bl: f64) -> Self {
        self.corner_radii = Some(RoundedRectRadii::new(tl, tr, br, bl));
        self
    }

    /// Proyecta una sombra detrás del nodo (drop shadow), rasterizada con
    /// el blur gaussiano nativo de vello. Se pinta antes del relleno, así
    /// el fill opaco la tapa y la sombra asoma por el desenfoque/offset.
    /// El radio de la sombra sigue al del nodo (más el `spread`). Ver
    /// [`Shadow`] (`Shadow::soft(alpha, blur)` es el default tasteful).
    pub fn shadow(mut self, shadow: Shadow) -> Self {
        self.shadow = Some(shadow);
        self
    }

    /// Rellena el nodo con un **gradiente** en vez de un color sólido. El
    /// gradiente se autorea en el **cuadrado unidad** `[0,1]²` y el runtime
    /// lo mapea al rect del nodo (así no necesitás saber el tamaño al
    /// construir el `View`) — igual que `Alignment` relativo de Flutter.
    ///
    /// ```ignore
    /// use llimphi_ui::llimphi_raster::peniko::{Color, Gradient};
    /// use llimphi_ui::llimphi_raster::kurbo::Point;
    /// // vertical: arriba claro → abajo oscuro
    /// let g = Gradient::new_linear(Point::new(0.0, 0.0), Point::new(0.0, 1.0))
    ///     .with_stops([Color::from_rgba8(80,90,110,255), Color::from_rgba8(30,34,44,255)].as_slice());
    /// view.fill_gradient(g)
    /// ```
    ///
    /// Gana sobre `fill` como base; un `hover_fill` (color) lo sigue
    /// overrideando mientras el cursor está encima.
    pub fn fill_gradient(mut self, gradient: Gradient) -> Self {
        self.fill_gradient = Some(gradient);
        self
    }

    /// Dibuja un borde (stroke) sobre el contorno redondeado del nodo,
    /// inset media línea hacia adentro (el grosor queda dentro del rect).
    /// Reemplaza el viejo truco de envolver el nodo en un rect-padre del
    /// color del borde con padding de 1px.
    pub fn border(mut self, width: f64, color: Color) -> Self {
        self.border = Some(Border::new(width, color));
        self
    }

    pub fn text(mut self, content: impl Into<String>, size_px: f32, color: Color) -> Self {
        self.text = Some(TextSpec {
            content: content.into(),
            size_px,
            color,
            alignment: llimphi_text::Alignment::Center,
            italic: false,
            font_family: None,
            line_height: 1.2,
            weight: 400.0,
            max_lines: None,
            ellipsis: false,
            runs: None,
            underline: false,
            strikethrough: false,
            spans: None,
        });
        self
    }

    pub fn text_aligned(
        mut self,
        content: impl Into<String>,
        size_px: f32,
        color: Color,
        alignment: llimphi_text::Alignment,
    ) -> Self {
        self.text = Some(TextSpec {
            content: content.into(),
            size_px,
            color,
            alignment,
            italic: false,
            font_family: None,
            line_height: 1.2,
            weight: 400.0,
            max_lines: None,
            ellipsis: false,
            runs: None,
            underline: false,
            strikethrough: false,
            spans: None,
        });
        self
    }

    /// Como `text_aligned` pero con un flag `italic`. Si la fuente activa
    /// no tiene variante italic, parley aplica synthesizing.
    pub fn text_aligned_italic(
        mut self,
        content: impl Into<String>,
        size_px: f32,
        color: Color,
        alignment: llimphi_text::Alignment,
        italic: bool,
    ) -> Self {
        self.text = Some(TextSpec {
            content: content.into(),
            size_px,
            color,
            alignment,
            italic,
            font_family: None,
            line_height: 1.2,
            weight: 400.0,
            max_lines: None,
            ellipsis: false,
            runs: None,
            underline: false,
            strikethrough: false,
            spans: None,
        });
        self
    }

    /// Como `text_aligned_italic` pero con font-family explícito.
    /// La cadena se pasa como `parley::FontStack::Source` (acepta listas
    /// CSS con fallbacks).
    pub fn text_aligned_full(
        mut self,
        content: impl Into<String>,
        size_px: f32,
        color: Color,
        alignment: llimphi_text::Alignment,
        italic: bool,
        font_family: Option<String>,
    ) -> Self {
        self.text = Some(TextSpec {
            content: content.into(),
            size_px,
            color,
            alignment,
            italic,
            font_family,
            line_height: 1.2,
            weight: 400.0,
            max_lines: None,
            ellipsis: false,
            runs: None,
            underline: false,
            strikethrough: false,
            spans: None,
        });
        self
    }

    /// Texto **multicolor** en una sola pasada de shaping: `content` se pinta
    /// con `default_color` y cada `(start_byte, end_byte, color)` de `runs`
    /// sobreescribe su rango (offsets en bytes). Pensado para syntax
    /// highlighting — un nodo por línea en vez de uno por token. Anclado
    /// arriba-izquierda (sin centrado vertical); el caller dimensiona el rect.
    pub fn text_runs(
        mut self,
        content: impl Into<String>,
        size_px: f32,
        default_color: Color,
        runs: Vec<(usize, usize, Color)>,
        alignment: llimphi_text::Alignment,
    ) -> Self {
        self.text = Some(TextSpec {
            content: content.into(),
            size_px,
            color: default_color,
            alignment,
            italic: false,
            font_family: None,
            line_height: 1.2,
            weight: 400.0,
            max_lines: None,
            ellipsis: false,
            runs: Some(runs),
            underline: false,
            strikethrough: false,
            spans: None,
        });
        self
    }

    /// Texto **RichText** (Bloque 13 de PARIDAD-FLUTTER, cierra Tier 2):
    /// `content` se pinta con los defaults del bloque (`size_px`,
    /// `default_color`, alignment, weight 400, no italic, line-height 1.2,
    /// fuente default) y cada [`llimphi_text::TextSpan`] sobreescribe en su
    /// rango de bytes uno o más de
    /// `size_px`/`weight`/`italic`/`font_family`/`color`/`underline`/
    /// `strikethrough`. Soporta wrap (el ancho lo fija el layout taffy del
    /// nodo); apto para párrafos con un `<b>`/`<i>`/`<code>`/`<small>`
    /// inline, links subrayados, headings dentro del mismo flujo, render
    /// barato de markdown.
    pub fn text_spans(
        mut self,
        content: impl Into<String>,
        size_px: f32,
        default_color: Color,
        spans: Vec<llimphi_text::TextSpan>,
        alignment: llimphi_text::Alignment,
    ) -> Self {
        self.text = Some(TextSpec {
            content: content.into(),
            size_px,
            color: default_color,
            alignment,
            italic: false,
            font_family: None,
            line_height: 1.2,
            weight: 400.0,
            max_lines: None,
            ellipsis: false,
            runs: None,
            underline: false,
            strikethrough: false,
            spans: Some(spans),
        });
        self
    }

    /// Adjunta o reemplaza los [`TextSpec::spans`] del texto ya seteado
    /// (RichText). Permite construir el texto con los builders uniformes
    /// (`.text_aligned(...).bold().underline()`) y luego inyectar overrides
    /// inline. No-op si el nodo no tiene texto.
    pub fn with_spans(mut self, spans: Vec<llimphi_text::TextSpan>) -> Self {
        if let Some(t) = self.text.as_mut() {
            t.spans = Some(spans);
        }
        self
    }

    /// Sobreescribe el múltiplo de interlínea del texto ya seteado (default
    /// 1.2). No-op si el nodo no tiene texto. Pensado para puriy, que pasa
    /// el `line-height` computado de CSS para que medición y pintado usen
    /// el mismo valor.
    pub fn line_height(mut self, mult: f32) -> Self {
        if let Some(t) = self.text.as_mut() {
            t.line_height = mult;
        }
        self
    }

    /// Sobreescribe el peso de fuente del texto ya seteado (default 400 =
    /// normal). Convención CSS: 400 normal, 500 medium, 600 semibold, 700
    /// bold. parley elige la variante más cercana de la familia activa o la
    /// sintetiza. No-op si el nodo no tiene texto. Afecta medida y pintado.
    pub fn text_weight(mut self, weight: f32) -> Self {
        if let Some(t) = self.text.as_mut() {
            t.weight = weight;
        }
        self
    }

    /// Atajo de [`Self::text_weight`] a 700 (bold). No-op sin texto.
    pub fn bold(self) -> Self {
        self.text_weight(700.0)
    }

    /// Fija la familia de fuente del texto ya seteado a la monoespaciada
    /// embebida ([`llimphi_text::MONOSPACE`]) — ancho fijo garantizado para
    /// que `ls`, tablas y logs columneen. No-op si el nodo no tiene texto.
    /// Afecta medida y pintado.
    pub fn mono(mut self) -> Self {
        if let Some(t) = self.text.as_mut() {
            t.font_family = Some(llimphi_text::MONOSPACE.to_string());
        }
        self
    }

    /// Clampa el texto a `n` líneas **sin** glifo de ellipsis (corte seco del
    /// prefijo que cupo). CSS `-webkit-line-clamp` sin `text-overflow`. No-op
    /// sin texto. Para el corte con `…` usar [`Self::ellipsis`]. Sólo trunca si
    /// hay envoltura (requiere ancho acotado por el layout).
    pub fn max_lines(mut self, n: usize) -> Self {
        if let Some(t) = self.text.as_mut() {
            t.max_lines = Some(n);
            t.ellipsis = false;
        }
        self
    }

    /// Clampa el texto a `n` líneas terminando la última en `…` cuando excede
    /// (CSS `text-overflow: ellipsis` + `-webkit-line-clamp: n`). Lo más común
    /// para items de lista, celdas de tabla, breadcrumbs y labels en cajas
    /// dimensionadas. `n = 1` es el clásico single-line ellipsis. No-op sin
    /// texto.
    pub fn ellipsis(mut self, n: usize) -> Self {
        if let Some(t) = self.text.as_mut() {
            t.max_lines = Some(n.max(1));
            t.ellipsis = true;
        }
        self
    }

    /// Activa subrayado del texto (CSS `text-decoration: underline` / Flutter
    /// `TextDecoration.underline`). parley registra la decoración por run y el
    /// runtime pinta la línea bajo la base usando `underline_offset` y
    /// `underline_size` del font metric — proporcional al tamaño de fuente
    /// elegido. No-op sin texto.
    pub fn underline(mut self) -> Self {
        if let Some(t) = self.text.as_mut() {
            t.underline = true;
        }
        self
    }

    /// Activa tachado del texto (CSS `text-decoration: line-through` /
    /// Flutter `TextDecoration.lineThrough`). Mismo régimen que [`Self::underline`]
    /// pero usando el strikethrough metric. No-op sin texto.
    pub fn strikethrough(mut self) -> Self {
        if let Some(t) = self.text.as_mut() {
            t.strikethrough = true;
        }
        self
    }

    pub fn on_click(mut self, msg: Msg) -> Self {
        self.on_click = Some(msg);
        self
    }

    /// Dispatch `msg` cuando el cursor entra al rect del nodo
    /// (transición no-hover → hover). Sólo emite una vez por entrada —
    /// el runtime no repite el msg si el cursor se mueve dentro del rect.
    pub fn on_pointer_enter(mut self, msg: Msg) -> Self {
        self.on_pointer_enter = Some(msg);
        self
    }

    /// Dispatch `msg` cuando el cursor sale del rect del nodo.
    pub fn on_pointer_leave(mut self, msg: Msg) -> Self {
        self.on_pointer_leave = Some(msg);
        self
    }

    /// Handler de **movimiento del cursor** sobre el nodo. Recibe `(local_x,
    /// local_y, rect_w, rect_h)` (posición relativa al rect del nodo) en CADA
    /// `CursorMoved` mientras el cursor está encima — no sólo al entrar, a
    /// diferencia de [`Self::on_pointer_enter`]. Útil para seguir el cursor:
    /// thumbnail de hover sobre un timeline, drawer que reacciona a la posición.
    /// Devolver `None` no dispara update.
    pub fn on_pointer_move_at<F>(mut self, handler: F) -> Self
    where
        F: Fn(f32, f32, f32, f32) -> Option<Msg> + Send + Sync + 'static,
    {
        self.on_pointer_move_at = Some(Arc::new(handler));
        self
    }

    /// Como `on_click`, pero el handler recibe `(local_x, local_y,
    /// rect_w, rect_h)` — la posición del cursor relativa al rect del
    /// nodo más las dimensiones actuales del nodo. Útil para canvas
    /// elements que necesitan saber dónde fue el click para convertirlo
    /// a coordenadas de mundo. Sobrescribe `on_click` para este nodo
    /// si ambos están presentes.
    pub fn on_click_at<F>(mut self, handler: F) -> Self
    where
        F: Fn(f32, f32, f32, f32) -> Option<Msg> + Send + Sync + 'static,
    {
        self.on_click_at = Some(Arc::new(handler));
        self
    }

    /// Declara el `Msg` a emitir cuando el usuario hace click derecho
    /// sobre este nodo. Para menús contextuales, conviene pasar un
    /// `Msg::OpenMenu { ... }` y dejar que el modelo guarde la
    /// posición; el overlay se abre vía [`App::view_overlay`].
    pub fn on_right_click(mut self, msg: Msg) -> Self {
        self.on_right_click = Some(msg);
        self
    }

    /// Variante posicional de [`Self::on_right_click`]. El handler recibe
    /// `(local_x, local_y, rect_w, rect_h)` para que un nodo "grilla"
    /// pueda resolver internamente qué subcelda recibió el click. La
    /// posición está relativa al rect del nodo.
    pub fn on_right_click_at<F>(mut self, handler: F) -> Self
    where
        F: Fn(f32, f32, f32, f32) -> Option<Msg> + Send + Sync + 'static,
    {
        self.on_right_click_at = Some(Arc::new(handler));
        self
    }

    /// Declara el `Msg` a emitir cuando el usuario hace click con el
    /// botón del medio (rueda presionada). Usado típicamente para abrir
    /// links en pestaña nueva — igual que Ctrl+Click pero más rápido.
    pub fn on_middle_click(mut self, msg: Msg) -> Self {
        self.on_middle_click = Some(msg);
        self
    }

    /// Pinta `image` dentro del rect del nodo. El encaje default es
    /// [`ImageFit::Contain`] (preservar aspect ratio cabiendo);
    /// usar [`Self::image_fit`] para `Cover`/`Fill`/`None`. El clip
    /// respeta `radius`/`corner_radii`, así avatares y cards
    /// redondeadas funcionan sin envolver en `clip(true)`. Re-exporta
    /// `peniko::Image` vía `llimphi_raster::peniko::Image` — el
    /// caller decodifica los bytes con el crate `image` (u otro) y
    /// construye el `Image` con `Blob<u8>` + `ImageFormat::Rgba8`.
    pub fn image(mut self, image: Image) -> Self {
        self.image = Some(image);
        self
    }

    /// Política de encaje de la imagen (CSS `object-fit` / Flutter
    /// `BoxFit`). Solo aplica si hay [`Self::image`] seteada. Ver
    /// [`ImageFit`].
    pub fn image_fit(mut self, fit: ImageFit) -> Self {
        self.image_fit = Some(fit);
        self
    }

    /// Registra una closure de pintura custom. El runtime la invoca
    /// con `(&mut vello::Scene, &mut Typesetter, PaintRect)` durante
    /// el paint del nodo. La closure es responsable de pintar
    /// primitivas custom dentro del rect; no debe dejar `push_layer`
    /// sin par. Soporte para canvas elements estilo
    /// dominium/pluma/cosmos.
    pub fn paint_with<F>(mut self, painter: F) -> Self
    where
        F: Fn(&mut vello::Scene, &mut llimphi_text::Typesetter, PaintRect)
            + Send
            + Sync
            + 'static,
    {
        self.painter = Some(Arc::new(painter));
        self
    }

    /// Registra una closure de pintura GPU directo. La closure recibe
    /// `(&Device, &Queue, &mut CommandEncoder, &TextureView, PaintRect, (viewport_w, viewport_h))`
    /// y debe escribir sobre el `TextureView` con `LoadOp::Load` (no
    /// clear) para preservar la pasada vello previa. El último
    /// argumento es el tamaño en pixels de la `TextureView` destino
    /// (la intermedia del frame) — necesario para calcular NDC sin
    /// asumir un viewport fijo. Ver [`GpuPaintFn`] para semántica
    /// completa, contexto y orden de pintura.
    pub fn gpu_paint_with<F>(mut self, painter: F) -> Self
    where
        F: Fn(
                &wgpu::Device,
                &wgpu::Queue,
                &mut wgpu::CommandEncoder,
                &wgpu::TextureView,
                PaintRect,
                (u32, u32),
            ) + Send
            + Sync
            + 'static,
    {
        self.gpu_painter = Some(Arc::new(painter));
        self
    }

    /// Recorta los hijos al rect de este nodo (paint y hit-test). Útil
    /// para paneles con contenido virtualizado que no debe sangrar a
    /// vecinos (listas, scrollers, viewers).
    pub fn clip(mut self, enabled: bool) -> Self {
        self.clip = enabled;
        self
    }

    /// Recorta los descendientes a un rect encogido por `insets` px
    /// `[top, right, bottom, left]` desde el rect del nodo — modela
    /// `clip-path: inset(...)`. Activa el recorte (paint + hit-test).
    pub fn clip_inset(mut self, insets: [f32; 4]) -> Self {
        self.clip = true;
        self.clip_inset = Some(insets);
        self
    }

    /// Recorta los descendientes a una elipse — modela
    /// `clip-path: circle()`/`ellipse()`. `spec` es de 14 floats: centro
    /// `[cx_px, cx_pct, cy_px, cy_pct]` + dos radios `[px, pct_w, pct_h,
    /// pct_diag, side]`, todos resueltos contra el rect del nodo en el
    /// pintado. Activa el recorte (paint; hit-test usa el rect completo).
    pub fn clip_ellipse(mut self, spec: [f32; 14]) -> Self {
        self.clip = true;
        self.clip_ellipse = Some(spec);
        self
    }

    pub fn children(mut self, children: Vec<View<Msg>>) -> Self {
        self.children = children;
        self
    }
}

#[cfg(test)]
mod semantics_tests {
    use super::*;
    use llimphi_layout::Style;

    #[test]
    fn clip_inset_setea_campo_y_activa_clip() {
        // `.clip_inset(...)` guarda los insets y activa el recorte (Fase 7.1219).
        let v = View::<()>::new(Style::default()).clip_inset([1.0, 2.0, 3.0, 4.0]);
        assert_eq!(v.clip_inset, Some([1.0, 2.0, 3.0, 4.0]));
        assert!(v.clip, "clip_inset implica clip activo");
        // `.clip(true)` solo (overflow:hidden) deja clip_inset en None.
        let h = View::<()>::new(Style::default()).clip(true);
        assert!(h.clip);
        assert_eq!(h.clip_inset, None);
        // Default: sin recorte.
        let d = View::<()>::new(Style::default());
        assert!(!d.clip);
        assert_eq!(d.clip_inset, None);
    }

    #[test]
    fn clip_ellipse_setea_campo_y_activa_clip() {
        // `.clip_ellipse(...)` guarda el spec de 14 floats y activa el recorte
        // (Fase 7.1220 rect, 7.1221 radios %, 7.1222 lados).
        let spec =
            [0.0, 50.0, 0.0, 50.0, 30.0, 0.0, 0.0, 0.0, 0.0, 20.0, 0.0, 0.0, 0.0, 0.0];
        let v = View::<()>::new(Style::default()).clip_ellipse(spec);
        assert_eq!(v.clip_ellipse, Some(spec));
        assert!(v.clip, "clip_ellipse implica clip activo");
        // No interfiere con clip_inset (campos independientes).
        assert_eq!(v.clip_inset, None);
        // Default: sin elipse.
        let d = View::<()>::new(Style::default());
        assert_eq!(d.clip_ellipse, None);
    }

    #[test]
    fn aria_label_sobre_role_preserva_role() {
        let v = View::<()>::new(Style::default())
            .role(Role::Button)
            .aria_label("Guardar");
        let s = v.semantics.expect("semantics");
        assert_eq!(s.role, Some(Role::Button));
        assert_eq!(s.label.as_deref(), Some("Guardar"));
    }

    #[test]
    fn role_sobre_aria_label_preserva_label() {
        // Orden invertido: el segundo setter no debe pisar lo del primero.
        let v = View::<()>::new(Style::default())
            .aria_label("Buscar")
            .role(Role::TextInput);
        let s = v.semantics.expect("semantics");
        assert_eq!(s.role, Some(Role::TextInput));
        assert_eq!(s.label.as_deref(), Some("Buscar"));
    }

    #[test]
    fn flags_independientes_no_se_pisan() {
        let v = View::<()>::new(Style::default())
            .role(Role::Checkbox)
            .aria_checked(true)
            .aria_required(true);
        let s = v.semantics.expect("semantics");
        assert_eq!(s.flags.checked, Some(true));
        assert_eq!(s.flags.required, Some(true));
        assert!(s.flags.disabled.is_none(), "no se setea lo que no se pidió");
    }

    #[test]
    fn semantics_spec_completo_reemplaza_lo_acumulado() {
        // `.semantics(spec)` es el setter "todo o nada"; debe sobrescribir.
        let v = View::<()>::new(Style::default())
            .role(Role::Button)
            .aria_label("Vieja")
            .semantics(SemanticsSpec::role(Role::Link).with_label("Nueva"));
        let s = v.semantics.expect("semantics");
        assert_eq!(s.role, Some(Role::Link));
        assert_eq!(s.label.as_deref(), Some("Nueva"));
    }
}
