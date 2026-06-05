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
            painter: None,
            gpu_painter: None,
            on_pointer_enter: None,
            on_pointer_leave: None,
            on_click: None,
            on_click_at: None,
            on_right_click: None,
            on_right_click_at: None,
            on_middle_click: None,
            drag: None,
            drag_at: None,
            drag_payload: None,
            on_drop: None,
            drop_hover_fill: None,
            clip: false,
            on_scroll: None,
            focusable: None,
            alpha: None,
            anim: None,
            transform: None,
            tooltip: None,
            cursor: None,
            children: Vec::new(),
        }
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

    /// Marca este nodo como enfocable con el id opaco `id`. El runtime lo
    /// incluye en el orden de Tab (pre-orden del árbol) y le da foco al
    /// clickearlo; cada cambio de foco se notifica vía `App::on_focus`.
    /// El caller pinta el focus-ring comparando el id contra el foco que
    /// guardó en su `Model`.
    pub fn focusable(mut self, id: u64) -> Self {
        self.focusable = Some(id);
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
        self.anim = Some(Anim { key, duration, easing: ease_out_cubic, enter: false, exit: false });
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
        self.anim = Some(Anim { key, duration, easing: ease_out_cubic, enter: true, exit: false });
        self
    }

    /// **Anima la salida** (fade-out): cuando esta `key` desaparece del árbol,
    /// el runtime retiene la última subescena que pintó y la reproduce con
    /// opacidad decreciente durante `duration` — estilo `AnimatedSwitcher` /
    /// `AnimatedVisibility` al ocultarse. No anima la entrada (para ambas, ver
    /// [`Self::animated_inout`]). Tiene coste por frame mientras el nodo vive
    /// (captura su subárbol); usar con moderación (toasts, modales, paneles).
    pub fn animated_exit(mut self, key: u64, duration: std::time::Duration) -> Self {
        self.anim = Some(Anim { key, duration, easing: ease_out_cubic, enter: false, exit: true });
        self
    }

    /// Anima **entrada y salida**: fade-in en la primera aparición y fade-out al
    /// desmontarse, ambos en `duration`. La pieza completa de "animación de
    /// contenido" para un nodo que aparece y desaparece (un toast, un panel que
    /// se abre y cierra, un resultado que entra y se va).
    pub fn animated_inout(mut self, key: u64, duration: std::time::Duration) -> Self {
        self.anim = Some(Anim { key, duration, easing: ease_out_cubic, enter: true, exit: true });
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
        self.anim = Some(Anim { key, duration, easing, enter: false, exit: false });
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
        });
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

    /// Pinta `image` dentro del rect del nodo, centrada y escalada
    /// preservando aspect ratio. Re-exporta `peniko::Image` vía
    /// `llimphi_raster::peniko::Image` — el caller decodifica los
    /// bytes con el crate `image` (u otro) y construye el `Image`
    /// con `Blob<u8>` + `ImageFormat::Rgba8`.
    pub fn image(mut self, image: Image) -> Self {
        self.image = Some(image);
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

    pub fn children(mut self, children: Vec<View<Msg>>) -> Self {
        self.children = children;
        self
    }
}
