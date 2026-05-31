use super::*;

impl<Msg> View<Msg> {
    pub fn new(style: Style) -> Self {
        Self {
            style,
            fill: None,
            hover_fill: None,
            radius: 0.0,
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
            alpha: None,
            children: Vec::new(),
        }
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

    pub fn text(mut self, content: impl Into<String>, size_px: f32, color: Color) -> Self {
        self.text = Some(TextSpec {
            content: content.into(),
            size_px,
            color,
            alignment: llimphi_text::Alignment::Center,
            italic: false,
            font_family: None,
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
            runs: Some(runs),
        });
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
                &llimphi_hal::wgpu::Device,
                &llimphi_hal::wgpu::Queue,
                &mut llimphi_hal::wgpu::CommandEncoder,
                &llimphi_hal::wgpu::TextureView,
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
