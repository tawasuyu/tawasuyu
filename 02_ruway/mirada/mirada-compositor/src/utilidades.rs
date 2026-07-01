// Funciones utilitarias del compositor.
use crate::*;
use smithay::input::keyboard::{xkb, ModifiersState, Keysym, Xkb};
use std::sync::Mutex;
use smithay::input::pointer::CursorImageSurfaceData;
use smithay::wayland::compositor::{with_states, with_surface_tree_downward, SurfaceAttributes, TraversalAction};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::SERIAL_COUNTER;
use auth_core::{ShellAction, UserInfo};

/// Código corto de la distribución de teclado **activa** para el indicador de
/// la barra. Con una sola distribución devuelve `""` (no hay nada que indicar y
/// el widget se oculta). Con varias, prefiere el código que el usuario puso en
/// `xkb_layout` (`csv`, p. ej. `"us,es,ru"`) indexado por el grupo activo —da
/// justo `"US"`/`"ES"`/`"RU"`—; si el índice no calza, cae al nombre humano que
/// reporta el keymap (`"Spanish"`…). `csv` es el `Config::xkb_layout` crudo.
pub(crate) fn short_layout(xkb: &Mutex<Xkb>, csv: &str) -> String {
    let g = match xkb.lock() {
        Ok(g) => g,
        Err(_) => return String::new(),
    };
    if g.layouts().count() <= 1 {
        return String::new();
    }
    let active = g.active_layout();
    let idx = active.0 as usize;
    let codes: Vec<&str> = csv.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    match codes.get(idx) {
        Some(code) => code.to_uppercase(),
        None => g.layout_name(active).to_string(),
    }
}

/// Construye la cadena de un atajo (`"Super+Shift+j"`) desde el estado de
/// modificadores y el keysym, con el mismo format que el mapa de teclas
/// de [`mirada_brain`]. `None` si no es una tecla mapeable.
pub(crate) fn combo_string(mods: &ModifiersState, sym: Keysym) -> Option<String> {
    let utf = xkb::keysym_to_utf8(sym);
    let key = utf.trim_end_matches('\0');
    let name = if key == " " {
        "space".to_string()
    } else {
        // ¿Es un único carácter imprimible? Entonces la tecla es ese carácter.
        let mut chars = key.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_graphic() => c.to_ascii_lowercase().to_string(),
            // Si no, una tecla con nombre: Return, Tab, Up, F5…
            _ => named_key(sym)?,
        }
    };
    let mut combo = String::new();
    if mods.logo {
        combo.push_str("Super+");
    }
    if mods.ctrl {
        combo.push_str("Ctrl+");
    }
    if mods.shift {
        combo.push_str("Shift+");
    }
    if mods.alt {
        combo.push_str("Alt+");
    }
    combo.push_str(&name);
    Some(combo)
}

/// Combos cableados que **siempre** cortan el compositor, estén o no en el
/// keymap y en cualquier modo —greeter incluido, donde los atajos del
/// escritorio no están registrados—. La red de seguridad para no quedar
/// varado: el clásico «zap» de X. Funciona igual en winit y en DRM.
pub(crate) fn is_escape_hatch(combo: &str) -> bool {
    matches!(combo, "Ctrl+Alt+BackSpace" | "Ctrl+Alt+Delete")
}

/// La VT destino de una conmutación de consola (`1` … `12`), o `None` si la
/// tecla no es de cambio de VT. Sólo lo honra el backend DRM —en winit no hay
/// VTs—. Es el comportamiento clásico para saltar entre consolas sin matar el
/// compositor.
///
/// Cubre los **dos** caminos, porque cuál llega depende del keymap activo:
/// 1. el keysym dedicado `XF86Switch_VT_n` (lo emiten los keymaps con la
///    sección `srvr_ctrl`, donde `Ctrl+Alt+Fn` ya no produce «Fn»); y
/// 2. `Ctrl+Alt+Fn` literal (keymaps base sin ese binding).
pub(crate) fn vt_target(mods: &ModifiersState, sym: Keysym) -> Option<i32> {
    let name = xkb::keysym_get_name(sym);
    // 1) Keysym dedicado: vale por sí mismo, sin exigir modificadores.
    if let Some(n) = name.strip_prefix("XF86Switch_VT_") {
        if let Ok(v) = n.parse::<i32>() {
            if (1..=12).contains(&v) {
                return Some(v);
            }
        }
    }
    // 2) Ctrl+Alt+Fn directo. Exigimos ambos modificadores para no conmutar
    //    con un F-key pelado.
    if mods.ctrl && mods.alt {
        if let Some(f) = name.strip_prefix('F') {
            if let Ok(v) = f.parse::<i32>() {
                if (1..=12).contains(&v) {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// Cierra el compositor y `exec`-uta una sesión ajena en su lugar, como el
/// usuario autenticado. Se llama **después** de salir del bucle y soltar el
/// DRM, así el compositor entrante (sway, Plasma…) puede tomar la GPU.
/// Reemplaza la imagen del proceso: si `exec` falla, registra y aborta.
pub(crate) fn exec_session(cmd: &str, as_user: Option<&UserInfo>) -> ! {
    use std::os::unix::process::CommandExt;
    println!("mirada-compositor · cediendo a la sesión: {cmd}");
    let mut command = std::process::Command::new("sh");
    command.arg("-c").arg(cmd).envs(THEME_ENV.iter().copied());
    if let Some(user) = as_user {
        if nix::unistd::geteuid().is_root() {
            // El compositor entrante crea su PROPIO socket Wayland, así que
            // necesita un XDG_RUNTIME_DIR suyo (no el de root, donde no
            // puede escribir) y no debe heredar nuestro WAYLAND_DISPLAY (el
            // DM ya cerró). Sin esto, Plasma/sway fallan con «could not
            // create wayland socket».
            use std::os::unix::fs::PermissionsExt;
            let xrd = format!("/run/user/{}", user.uid);
            let _ = std::fs::create_dir_all(&xrd);
            let _ = std::fs::set_permissions(&xrd, std::fs::Permissions::from_mode(0o700));
            let _ = nix::unistd::chown(
                xrd.as_str(),
                Some(nix::unistd::Uid::from_raw(user.uid)),
                Some(nix::unistd::Gid::from_raw(user.gid)),
            );
            command.env("XDG_RUNTIME_DIR", &xrd);
            command.env_remove("WAYLAND_DISPLAY");
            apply_user(&mut command, user);
        }
    }
    let err = command.exec(); // sólo retorna si falla
    dlog!("mirada-compositor · no pude ceder a «{cmd}»: {err}");
    std::process::exit(1);
}

/// El nombre canónico de una tecla especial — `Return`, `Tab`, `Up`,
/// `F5`… `None` si xkb no le da un nombre razonable.
pub(crate) fn named_key(sym: Keysym) -> Option<String> {
    let name = xkb::keysym_get_name(sym);
    if name.is_empty() || name == "NoSymbol" || name.starts_with("0x") {
        None
    } else {
        Some(name)
    }
}

/// Despacha frame callbacks a los popups (menús) colgados de `parent`, recursivo
/// (submenús incluidos). Sin esto un menú GTK/Qt pinta el primer cuadro y no
/// vuelve a repintar — el resaltado al pasar el mouse quedaría congelado.
pub(crate) fn send_frames_popups(parent: &WlSurface, time: u32) {
    for (popup, _) in smithay::desktop::PopupManager::popups_for_surface(parent) {
        let s = popup.wl_surface().clone();
        send_frames_surface_tree(&s, time);
        send_frames_popups(&s, time);
    }
}

/// Recolecta las callbacks de `wp_presentation` de un árbol de superficies hacia
/// `feedback`. El llamante ya filtró a superficies que ESTÁN en la salida de
/// `feedback`, así que el closure de «salida de scan-out primaria» siempre reporta
/// esa salida y todas se recolectan. No reclamamos zero-copy (flags vacías): es un
/// hint de optimización, omitirlo es correcto.
pub(crate) fn take_presentation_feedback_tree(
    surface: &WlSurface,
    feedback: &mut smithay::desktop::utils::OutputPresentationFeedback,
) {
    use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
    let output = feedback.output();
    smithay::desktop::utils::take_presentation_feedback_surface_tree(
        surface,
        feedback,
        |_, _| output.clone(),
        |_, _| wp_presentation_feedback::Kind::empty(),
    );
}

/// Como [`take_presentation_feedback_tree`] pero para los popups (menús) colgados
/// de `parent`, recursivo — mismo recorrido que [`send_frames_popups`].
pub(crate) fn take_presentation_feedback_popups(
    parent: &WlSurface,
    feedback: &mut smithay::desktop::utils::OutputPresentationFeedback,
) {
    for (popup, _) in smithay::desktop::PopupManager::popups_for_surface(parent) {
        let s = popup.wl_surface().clone();
        take_presentation_feedback_tree(&s, feedback);
        take_presentation_feedback_popups(&s, feedback);
    }
}

/// Despacha los callbacks de frame de un árbol de superficies: avisa a
/// cada cliente de que puede dibujar el siguiente cuadro.
pub(crate) fn send_frames_surface_tree(surface: &WlSurface, time: u32) {
    with_surface_tree_downward(
        surface,
        (),
        |_, _, &()| TraversalAction::DoChildren(()),
        |_surf, states, &()| {
            for callback in states
                .cached_state
                .get::<SurfaceAttributes>()
                .current()
                .frame_callbacks
                .drain(..)
            {
                callback.done(time);
            }
        },
        |_, _, &()| true,
    );
}

/// Dónde pintar una ventana. La del shell se ancla al pie de la salida
/// y crece hacia arriba (su cajón de resultados se despliega sobre las
/// ventanas). Una ventana normal va en su celda; si el cliente presenta
/// una superficie más pequeña que la celda (p. ej. un terminal que
/// redondea su tamaño a celdas de texto), se centra en el hueco.
/// Elementos de render de los layer surfaces de la salida, separados en
/// `(encima, debajo)` de las ventanas: `encima` = capas Overlay+Top,
/// `debajo` = Bottom+Background. Cada layer se pinta en la geometría que
/// el `LayerMap` le calculó (anclaje + márgenes). Coordenadas top-left,
/// igual que las ventanas. Lo comparten los backends winit y DRM.
pub(crate) fn layer_render_elements(
    output: Option<&Output>,
    renderer: &mut GlesRenderer,
) -> (
    Vec<WaylandSurfaceRenderElement<GlesRenderer>>,
    Vec<WaylandSurfaceRenderElement<GlesRenderer>>,
) {
    let mut over = Vec::new();
    let mut under = Vec::new();
    let Some(output) = output else {
        return (over, under);
    };
    let map = layer_map_for_output(output);
    for layer in map.layers() {
        let Some(geo) = map.layer_geometry(layer) else {
            continue;
        };
        if !buffer_render_sano(layer.wl_surface()) {
            continue; // buffer degenerado/desmesurado: no lo importamos
        }
        let els = render_elements_from_surface_tree(
            renderer,
            layer.wl_surface(),
            (geo.loc.x, geo.loc.y),
            1.0,
            1.0,
            Kind::Unspecified,
        );
        match layer.layer() {
            Layer::Overlay | Layer::Top => over.extend(els),
            Layer::Background | Layer::Bottom => under.extend(els),
        }
    }
    (over, under)
}

/// Geometrías (x, y, w, h en coords **locales** a la salida) de los paneles
/// layer-shell de **arriba** (Overlay/Top) de `output` — los que el glass puede
/// dejar *frosted* (un bar tipo waybar, o la propia `pata` de mirada, que es un
/// layer Top). Mismas coords que [`layer_render_elements`] usa para pintarlos.
/// Sólo los de buffer sano. Vacío sin salida o sin paneles.
pub(crate) fn over_layer_rects(output: Option<&Output>) -> Vec<(i32, i32, i32, i32)> {
    let mut v = Vec::new();
    let Some(output) = output else {
        return v;
    };
    let map = layer_map_for_output(output);
    for layer in map.layers() {
        if !matches!(layer.layer(), Layer::Overlay | Layer::Top) {
            continue;
        }
        let Some(geo) = map.layer_geometry(layer) else {
            continue;
        };
        if !buffer_render_sano(layer.wl_surface()) {
            continue;
        }
        v.push((geo.loc.x, geo.loc.y, geo.size.w, geo.size.h));
    }
    v
}

/// La transformada de la **lupa** (zoom de pantalla completa — accesibilidad
/// para hipermétropes). Dado el tamaño de la salida `size = (w, h)` en px, el
/// punto focal `(fx, fy)` —el puntero, en coords **locales** a la salida— y el
/// `factor` (>1), devuelve `(origin, scale)` para envolver cada elemento
/// compuesto en `RescaleRenderElement::from_element(el, origin, scale)`: el
/// reescalado fija `origin` y multiplica las distancias por `scale`, así que toda
/// la escena se agranda alrededor del puntero.
///
/// El foco se **acota** para que la región magnificada (de tamaño `(w/factor,
/// h/factor)`) quepa entera en la salida: pegado a un borde el puntero deja de
/// estar centrado —no se puede paneár más allá del borde—, como toda lupa real.
/// Con `factor <= 1` o salida degenerada devuelve la identidad (`origin` (0,0),
/// `scale` 1.0). Pura y testeada.
pub(crate) fn magnify_origin(
    size: (i32, i32),
    focal: (f64, f64),
    factor: f32,
) -> (Point<i32, smithay::utils::Physical>, f64) {
    let (w, h) = (size.0 as f64, size.1 as f64);
    let z = factor as f64;
    if z <= 1.0 || w <= 0.0 || h <= 0.0 {
        return (Point::from((0, 0)), 1.0);
    }
    // Por eje: acota el foco a `[media-ventana, largo - media-ventana]` y resuelve
    // `origin` de `centro = origin + (foco - origin)·z`, que da `origin ∈ [0, largo]`.
    let axis = |f: f64, len: f64| -> f64 {
        let half = len / (2.0 * z);
        let centro = len / 2.0;
        let fc = f.clamp(half, len - half);
        ((centro - fc * z) / (1.0 - z)).clamp(0.0, len)
    };
    let ox = axis(focal.0, w).round() as i32;
    let oy = axis(focal.1, h).round() as i32;
    (Point::from((ox, oy)), z)
}

/// La ruta por defecto de una **grabación de pantalla**:
/// `~/Videos/mirada-<epoch>.<ext>` (crea `~/Videos` si falta). El timestamp en
/// segundos epoch hace el nombre único sin depender de `chrono`. `ext` = `"mp4"`
/// / `"webm"` según el códec. Si no hay `HOME`, cae al directorio actual.
pub(crate) fn default_record_path(ext: &str) -> std::path::PathBuf {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let dir = home.join("Videos");
    let _ = std::fs::create_dir_all(&dir);
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    dir.join(format!("mirada-{secs}.{ext}"))
}

/// Ancho (px) de cada botón del titlebar. Compartido entre el render y el
/// hit-test del click.
pub(crate) const TB_BTN_W: i32 = 28;

/// Una celda de la barra de título: un botón de **sistema** (el item del layout,
/// siempre un `Button`) o uno **aportado por una app** mirada-aware (id + glifo).
pub(crate) enum TbCell<'a> {
    Sys(&'a mirada_brain::TitlebarItem),
    App { item_id: &'a str, glyph: &'a str },
}

impl TbCell<'_> {
    /// La acción de un botón de sistema (`None` para los aportados por apps).
    pub(crate) fn action(&self) -> Option<&mirada_brain::TitlebarAction> {
        match self {
            TbCell::Sys(mirada_brain::TitlebarItem::Button { action, .. }) => Some(action),
            _ => None,
        }
    }
}

/// El nº efectivo de botones a izquierda y derecha = los de sistema (sólo los
/// `Button` del layout; los `App` reservados no cuentan) + las contribuciones de
/// la app por lado. Lo comparten el cálculo de celdas y el rango del título.
fn effective_counts(
    layout: &mirada_brain::TitlebarLayout,
    contribs: &[mirada_aware::AwareItem],
) -> (i32, i32) {
    let es_boton = |it: &mirada_brain::TitlebarItem| matches!(it, mirada_brain::TitlebarItem::Button { .. });
    let lc = layout.left.iter().filter(|i| es_boton(i)).count() as i32
        + contribs.iter().filter(|c| c.side == mirada_aware::AwareSide::Left).count() as i32;
    let rc = layout.right.iter().filter(|i| es_boton(i)).count() as i32
        + contribs.iter().filter(|c| c.side == mirada_aware::AwareSide::Right).count() as i32;
    (lc, rc)
}

/// Las celdas `(x_izquierdo, celda)` del titlebar de una ventana cuyo contenido
/// ocupa `cx..cx+cw`, combinando los botones de **sistema** del `layout` con las
/// **contribuciones** de la app (`contribs`). Grupo izquierdo: sistema-izq +
/// contribuciones-izq, apilados desde `cx`. Grupo derecho: contribuciones-der +
/// sistema-der, terminando en el borde derecho (los de sistema quedan pegados al
/// borde: cerrar/maximizar nunca los desplaza una contribución). Omite lo que no
/// entra. La usan **por igual** el render y el hit-test, así dibujo y click nunca
/// divergen.
pub(crate) fn titlebar_cells_for<'a>(
    layout: &'a mirada_brain::TitlebarLayout,
    contribs: &'a [mirada_aware::AwareItem],
    cx: i32,
    cw: i32,
) -> Vec<(i32, TbCell<'a>)> {
    use mirada_aware::AwareSide;
    use mirada_brain::TitlebarItem;
    // Items efectivos por grupo, en orden de pintado.
    let mut left: Vec<TbCell> = Vec::new();
    for it in &layout.left {
        if matches!(it, TitlebarItem::Button { .. }) {
            left.push(TbCell::Sys(it));
        }
    }
    for c in contribs.iter().filter(|c| c.side == AwareSide::Left) {
        left.push(TbCell::App { item_id: &c.id, glyph: &c.glyph });
    }
    let mut right: Vec<TbCell> = Vec::new();
    for c in contribs.iter().filter(|c| c.side == AwareSide::Right) {
        right.push(TbCell::App { item_id: &c.id, glyph: &c.glyph });
    }
    for it in &layout.right {
        if matches!(it, TitlebarItem::Button { .. }) {
            right.push(TbCell::Sys(it));
        }
    }
    // Posicionar: izquierda left-to-right desde cx; derecha left-to-right
    // terminando en el borde derecho.
    let mut out = Vec::new();
    let mut lx = cx;
    for cell in left {
        if lx + TB_BTN_W > cx + cw {
            break;
        }
        out.push((lx, cell));
        lx += TB_BTN_W;
    }
    let n = right.len() as i32;
    for (k, cell) in right.into_iter().enumerate() {
        let x = cx + cw - (n - k as i32) * TB_BTN_W;
        if x < lx {
            continue;
        }
        out.push((x, cell));
    }
    out
}

/// El rango horizontal `(inicio, fin)` disponible para el **título**: entre el
/// final del grupo izquierdo y el comienzo del derecho (contribuciones
/// incluidas), con un padding lateral. Para alinear el título sin pisar botones.
pub(crate) fn titlebar_title_range(
    layout: &mirada_brain::TitlebarLayout,
    contribs: &[mirada_aware::AwareItem],
    cx: i32,
    cw: i32,
) -> (i32, i32) {
    let pad = 8;
    let (left_n, right_n) = effective_counts(layout, contribs);
    let start = cx + left_n * TB_BTN_W + pad;
    let end = cx + cw - right_n * TB_BTN_W - pad;
    (start, end.max(start))
}

/// Las imágenes de `dir` aptas como wallpaper (png/jpg/jpeg/webp/bmp),
/// ordenadas por nombre. Para el fondo automático (slideshow).
pub(crate) fn list_wallpaper_images(dir: &str) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
                if matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "png" | "jpg" | "jpeg" | "webp" | "bmp"
                ) {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out
}

/// El alto efectivo de la barra de título de `w`: `0` para el shell, las
/// ventanas a pantalla completa y las que se decoran solas (CSD, `!w.ssd`) —
/// no llevan barra del servidor—; `0` también para las **teseladas** cuando el
/// perfil pide barra sólo-en-flotantes (`w.titlebar_floating_only && !w.floating`);
/// el `titlebar_height` configurado para el resto. Acotado a `>= 0`. Es el gate
/// único: render, `render_loc`, el hit-test del input y el `configure` del
/// cliente pasan todos por acá, así que apagar la barra de una ventana se
/// propaga consistente.
pub(crate) fn titlebar_for(w: &ManagedWindow, titlebar_height: i32) -> i32 {
    if w.is_shell || w.fullscreen || w.is_greeter || !w.ssd {
        0
    } else if w.titlebar_floating_only && !w.floating {
        // Estilo tiling: las teseladas son cuerpo entero; sólo las flotantes
        // (z-order) llevan barra agarrable.
        0
    } else {
        titlebar_height.max(0)
    }
}

/// La posición de la **superficie** del cliente. `titlebar_height` reserva esa
/// franja arriba de la celda (la superficie baja por `tb`); el resto centra la
/// superficie en el área de contenido si el cliente presenta algo más chico.
pub(crate) fn render_loc(w: &ManagedWindow, output_h: i32, titlebar_height: i32) -> (i32, i32) {
    if w.is_shell {
        // Sólo el anclaje inferior crece hacia arriba cuando el cliente
        // presenta una superficie más alta que la franja (cajón desplegado);
        // los demás bordes usan la posición acoplada tal cual.
        if shell_dock().anchor == ShellAnchor::Bottom {
            let h = surface_px_size(w).map(|(_, h)| h).unwrap_or(shell_dock().thickness);
            return (0, output_h - h);
        }
        return w.loc;
    }
    let tb = titlebar_for(w, titlebar_height);
    let content_top = w.loc.1 + tb;
    let content_h = (w.size.1 - tb).max(1);
    // El tamaño VISIBLE (contenido sin la sombra CSD) centra la ventana; el
    // `offset` de la sombra corre el buffer hacia atrás para que el CONTENIDO
    // —no el borde del buffer— caiga en la celda. La sombra rebalsa hacia afuera
    // (translúcida, no estorba). Sin geometría declarada esto es idéntico al
    // comportamiento previo (offset 0, tamaño = buffer). Devuelve el origen del
    // BUFFER a dibujar; el contenido queda en `(origen + offset)`.
    match content_px_size(w) {
        Some((cw, ch)) => {
            let (off_x, off_y) = content_offset(w);
            let dx = ((w.size.0 - cw) / 2).max(0);
            let dy = ((content_h - ch) / 2).max(0);
            (w.loc.0 + dx - off_x, content_top + dy - off_y)
        }
        _ => (w.loc.0, content_top),
    }
}

/// El tamaño en píxeles de la superficie de una ventana, si el cliente
/// ya presentó un buffer. `None` mientras no haya dibujado nada — la usa
/// el backend DRM para acertar el rectángulo en el test de impacto del
/// puntero.
pub(crate) fn surface_px_size(w: &ManagedWindow) -> Option<(i32, i32)> {
    with_renderer_surface_state(&w.surface, |s| s.surface_size())
        .flatten()
        .map(|s| (s.w, s.h))
}

/// El rectángulo de **geometría de ventana** declarado por el cliente
/// (`xdg_surface.set_window_geometry`): el contenido real dentro del buffer.
/// Las apps CSD (Firefox/Zen, GTK) meten una **sombra invisible** en el buffer
/// y declaran acá el recorte de contenido — `loc` es el desplazamiento de la
/// sombra (cuánto entra el contenido) y `size` el contenido sin sombra.
/// `None` si el cliente no la declaró o es nula (entonces contenido = buffer).
pub(crate) fn window_geometry(
    surface: &WlSurface,
) -> Option<smithay::utils::Rectangle<i32, smithay::utils::Logical>> {
    with_states(surface, |states| {
        states
            .cached_state
            .get::<smithay::wayland::shell::xdg::SurfaceCachedState>()
            .current()
            .geometry
    })
    .filter(|g| g.size.w > 0 && g.size.h > 0)
}

/// El desplazamiento `(x, y)` de la sombra CSD dentro del buffer (el `loc` de
/// la geometría), o `(0, 0)` si no hay geometría. El buffer se dibuja corrido
/// por **`-offset`** para que el contenido —no la sombra— caiga en la celda.
pub(crate) fn content_offset(w: &ManagedWindow) -> (i32, i32) {
    window_geometry(&w.surface)
        .map(|g| (g.loc.x, g.loc.y))
        .unwrap_or((0, 0))
}

/// El tamaño **visible** de la ventana: el contenido declarado por geometría
/// (sin la sombra CSD) si lo hay; si no, el buffer entero. Lo que deben abrazar
/// el marco y la barra de mirada y el hit-test, no el buffer-con-sombra.
pub(crate) fn content_px_size(w: &ManagedWindow) -> Option<(i32, i32)> {
    if let Some(g) = window_geometry(&w.surface) {
        return Some((g.size.w, g.size.h));
    }
    surface_px_size(w)
}

/// Tope de lado para el buffer raíz de una superficie a componer. Encima de
/// `GL_MAX_TEXTURE_SIZE` típico (16384) el driver rechaza el import igual; lo
/// atajamos antes para no pagar el intento ni su `warn` por frame —caro sobre
/// todo en el cursor, que se importa en cada vblank— ni malgastar VRAM en
/// texturas desmesuradas que un cliente malicioso podría adjuntar.
pub(crate) const MAX_SURFACE_PX: i32 = 16384;

/// `false` si el buffer raíz de `surface` es degenerado (lado ≤ 0) o desmesurado
/// (> [`MAX_SURFACE_PX`]); en ese caso el camino de composición saltea su árbol.
/// Sin buffer todavía → `true` (smithay no emite nada). No inspecciona
/// subsuperficies: ataja el caso dominante (un toplevel o cursor gigante), no el
/// árbol entero —la importación de smithay sigue cubriendo el resto (warn+skip).
pub(crate) fn buffer_render_sano(surface: &WlSurface) -> bool {
    match with_renderer_surface_state(surface, |s| s.surface_size()) {
        Some(Some(sz)) => sz.w > 0 && sz.h > 0 && sz.w <= MAX_SURFACE_PX && sz.h <= MAX_SURFACE_PX,
        _ => true,
    }
}

/// `true` si la superficie ya presentó un buffer (está **mapeada**). Antes del
/// primer commit con buffer smithay no tiene `renderer_surface_state`, así que
/// devuelve `false`. Lo usa el foco de teclado para no enfocar una ventana que
/// el cliente todavía no pintó (ver `App::pending_kb_focus`).
pub(crate) fn surface_mapeada(surface: &WlSurface) -> bool {
    with_renderer_surface_state(surface, |s| s.surface_size())
        .flatten()
        .is_some()
}

/// El punto caliente (hotspot) de una superficie de cursor: el píxel de
/// la imagen que debe quedar bajo la posición real del puntero. `(0, 0)`
/// si el cliente no lo declaró.
pub(crate) fn cursor_hotspot(surface: &WlSurface) -> (i32, i32) {
    with_states(surface, |states| {
        states
            .data_map
            .get::<CursorImageSurfaceData>()
            .map(|m| {
                let h = lock_tolerante(m).hotspot;
                (h.x, h.y)
            })
            .unwrap_or((0, 0))
    })
}

/// Variables de entorno de tema que el compositor inyecta a cada hijo,
/// para uniformizar GTK y Qt:
/// - `XDG_CURRENT_DESKTOP=mirada` hace que `xdg-desktop-portal` enrute
///   hacia `mirada-portal` (el backend de `org.freedesktop.appearance`).
/// - `QT_QPA_PLATFORMTHEME=gtk3` hace que las apps Qt sigan el tema GTK,
///   y por tanto el `gtk.css` que genera `nahual-theme`.
pub(crate) const THEME_ENV: &[(&str, &str)] = &[
    ("XDG_CURRENT_DESKTOP", "mirada"),
    ("QT_QPA_PLATFORMTHEME", "gtk3"),
];

/// Lanza un comando como proceso hijo, vía `sh -c`. El hijo hereda el
/// entorno —`WAYLAND_DISPLAY` incluido—, así que el cliente que abra se
/// conecta a este compositor; además se le inyecta [`THEME_ENV`] para
/// que GTK y Qt adopten el tema del escritorio. Lo usan la acción
/// `spawn:…` del keymap, la variable `MIRADA_STARTUP` y el autoarranque.
///
/// `as_user`: si viene una identidad y el compositor corre como root
/// (modo DM, tras el traspaso), el hijo baja a ese usuario — ver
/// [`apply_user`]. Con `None`, o sin ser root, lanza con la identidad
/// actual del compositor.
/// Convierte una entrada de config del menú en un nodo del árbol del menú
/// raíz: hoja si no tiene `submenu`, submenú (recursivo) si lo tiene.
pub(crate) fn menu_node_from_entry(e: &mirada_brain::MenuEntry) -> crate::menu::MenuNode {
    if e.submenu.is_empty() {
        crate::menu::MenuNode::leaf(e.label.clone(), e.command.clone())
    } else {
        crate::menu::MenuNode::submenu(
            e.label.clone(),
            e.submenu.iter().map(menu_node_from_entry).collect(),
        )
    }
}

/// Lanza `cmd` como el usuario de la sesión. Devuelve el **PID** del hijo cuando
/// lo spawnea por el camino crudo (para que el autoexec pueda terminarlo luego);
/// `None` si no hay comando, si falla, o si lo entregó a arje como Ente (modo
/// session-manager, donde el PID no es nuestro).
pub(crate) fn spawn_command(
    cmd: &str,
    as_user: Option<&UserInfo>,
    session_env: &[(String, String)],
) -> Option<u32> {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return None;
    }
    // Session-manager (opt-in, `MIRADA_SESSION_ENTES=1`): entregar la app a arje
    // como Ente supervisado + re-floorable en vez de spawnearla cruda. Default OFF
    // —y fallback al camino crudo si arje no la acepta— porque arje todavía corre
    // los Entes como root (sin drop de uid/gid); el `spawn_command` crudo SÍ hace
    // `setuid` al usuario. Pasa a default cuando arje gane `run_as`. Ver `session`.
    if crate::session::ente_mode() {
        let label = cmd.split_whitespace().next().unwrap_or("app");
        if crate::session::try_spawn_as_ente(label, cmd, as_user, session_env) {
            return None; // lo maneja arje; el PID no es nuestro
        }
    }
    let mut command = std::process::Command::new("sh");
    command.arg("-c").arg(cmd).envs(THEME_ENV.iter().copied());
    // Entorno de sesión (runtime dir del usuario, WAYLAND_DISPLAY absoluto,
    // bus D-Bus) — vacío para el greeter, poblado tras el traspaso.
    for (k, v) in session_env {
        command.env(k, v);
    }
    // Telemetría de sesión: capturá el stdout/stderr del app a
    // /var/log/arje/sesion-<app>.log para poder diagnosticar la sesión post-login
    // (pata, shuma, etc.). El compositor abre el archivo (root) y el fd sobrevive
    // el drop a `user`. Best-effort: sin permiso (dev no-root) hereda el stdio del
    // compositor como antes.
    if let Some(f) = open_sesion_log(cmd) {
        if let Ok(f2) = f.try_clone() {
            command.stdout(std::process::Stdio::from(f));
            command.stderr(std::process::Stdio::from(f2));
        }
    }
    if let Some(user) = as_user {
        if nix::unistd::geteuid().is_root() {
            apply_user(&mut command, user);
        }
    }
    match command.spawn() {
        Ok(child) => {
            let pid = child.id();
            println!("mirada-compositor · lanzado (pid {pid}): {cmd}");
            Some(pid)
        }
        Err(e) => {
            dlog!("mirada-compositor · no pude lanzar «{cmd}»: {e}");
            None
        }
    }
}

/// Abre el log de telemetría de un app de sesión: `/var/log/arje/sesion-<app>.log`
/// (append). Deriva el nombre del primer token del comando. `None` si no se puede
/// (sin permiso en dev no-root) → el app hereda el stdio del compositor.
fn open_sesion_log(cmd: &str) -> Option<std::fs::File> {
    let app = cmd.split_whitespace().next().unwrap_or("app");
    let base = app.rsplit('/').next().unwrap_or(app);
    let safe: String = base
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let safe = if safe.is_empty() { "app".to_string() } else { safe };
    let _ = std::fs::create_dir_all("/var/log/arje");
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(format!("/var/log/arje/sesion-{safe}.log"))
        .ok()
}

/// Prepara un `Command` para que el hijo corra como `user`: fija grupos
/// suplementarios, gid, uid y una sesión propia, hace `cd` a su home e
/// inyecta las variables de identidad. Sólo se llama tras comprobar que
/// el compositor es root.
///
/// La lista de grupos se calcula **en el padre**: `getgrouplist`
/// consulta NSS (abre `/etc/group`), y eso no es seguro entre `fork` y
/// `exec`; en `pre_exec` quedan sólo syscalls async-signal-safe.
pub(crate) fn apply_user(command: &mut std::process::Command, user: &UserInfo) {
    use nix::unistd::{setgid, setgroups, setuid, Gid, Uid};
    use std::os::unix::process::CommandExt;

    let uid = Uid::from_raw(user.uid);
    let gid = Gid::from_raw(user.gid);
    let groups: Vec<Gid> = std::ffi::CString::new(user.name.as_bytes())
        .ok()
        .and_then(|name| nix::unistd::getgrouplist(&name, gid).ok())
        .unwrap_or_else(|| vec![gid]);

    command
        .env("HOME", &user.home)
        .env("USER", &user.name)
        .env("LOGNAME", &user.name)
        .env("SHELL", &user.shell)
        .current_dir(&user.home);

    // SAFETY: corre en el hijo, entre `fork` y `exec`. Sólo syscalls
    // async-signal-safe. El orden es obligatorio: grupos y gid ANTES que
    // uid — al rebajar el uid se pierde el privilegio para fijarlos.
    unsafe {
        command.pre_exec(move || {
            setgroups(&groups)?;
            setgid(gid)?;
            setuid(uid)?;
            let _ = nix::unistd::setsid(); // sesión propia; no es crítico
            Ok(())
        });
    }
}

/// La ruta del archivo de autoarranque, `…/mirada/autostart` — junto al
/// keymap y las reglas. Con un usuario (tras el traspaso del DM) se
/// resuelve bajo su home; sin él, bajo la config del proceso actual.
pub(crate) fn autostart_path(user: Option<&UserInfo>) -> Option<std::path::PathBuf> {
    match user {
        Some(u) => Some(u.home.join(".config/mirada/autostart")),
        None => Keymap::default_path().and_then(|p| p.parent().map(|d| d.join("autostart"))),
    }
}

/// Lanza los programas del archivo de autoarranque: un comando por
/// línea, `#` comenta y las líneas en blanco se saltan. Sin archivo, no
/// hace nada. Se llama una vez al arrancar (o tras el traspaso del DM),
/// con el socket ya abierto. `as_user` se propaga a [`spawn_command`].
pub(crate) fn spawn_autostart(as_user: Option<&UserInfo>, session_env: &[(String, String)]) {
    let text = autostart_path(as_user)
        .and_then(|path| std::fs::read_to_string(&path).ok())
        .unwrap_or_default();
    let mut n = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        spawn_command(line, as_user, session_env);
        n += 1;
    }
    if n > 0 {
        println!("mirada-compositor · autoarranque: {n} programa(s).");
    } else {
        // Sin autostart: en vez de un escritorio negro y vacío, levanta el
        // marco pata para que haya algo usable de entrada.
        println!("mirada-compositor · sin autoarranque — levanto el marco pata.");
        spawn_command("pata-llimphi", as_user, session_env);
    }
}

/// La ruta del `config.ron` del usuario — junto al keymap y el autostart.
/// Con un usuario (tras el traspaso del DM) se resuelve bajo su home; sin él,
/// bajo la config del proceso actual.
pub(crate) fn config_path(user: Option<&UserInfo>) -> Option<std::path::PathBuf> {
    match user {
        Some(u) => Some(u.home.join(".config/mirada/config.ron")),
        None => mirada_brain::Config::default_path(),
    }
}

/// Lanza las apps del **autoarranque rico** de `config.ron` ([`startup`]): cada
/// entrada se resuelve a su comando de shell (envuelto en `waypipe ssh` si es
/// remota) y se lanza con [`spawn_command`]. La ubicación por escritorio la
/// resuelven las reglas derivadas de `startup` (ver `embedded_brain`). Es el
/// complemento del archivo `autostart` ([`spawn_autostart`]); ambos corren.
///
/// [`startup`]: mirada_brain::Config::startup
pub(crate) fn spawn_config_startup(as_user: Option<&UserInfo>, session_env: &[(String, String)]) {
    let Some(path) = config_path(as_user) else {
        return;
    };
    let Ok(cfg) = mirada_brain::Config::load(&path) else {
        return; // sin config o corrupta: nada que autoarrancar (ya avisó el brain)
    };
    let mut n = 0;
    for app in cfg.startup() {
        spawn_command(&app.shell_command(), as_user, session_env);
        n += 1;
    }
    if n > 0 {
        println!("mirada-compositor · autoarranque (config.ron): {n} app(s).");
    }
}

/// Nombre o ruta del binario del greeter. `MIRADA_GREETER_BIN` lo
/// sobreescribe — cómodo en desarrollo para apuntar a `target/…`.
pub(crate) fn greeter_bin() -> String {
    std::env::var("MIRADA_GREETER_BIN").unwrap_or_else(|_| "mirada-greeter".to_string())
}

/// Lanza `mirada-greeter` como proceso hijo (shell de credenciales) con el
/// stdout capturado. Un hilo lee sus líneas: la que sea una [`ShellAction`] se
/// entrega por `send` (el bucle de eventos la aplica — arrancar sesión o
/// desbloquear); el resto del stdout se reenvía a la consola con el prefijo
/// `greeter ·`. El hilo es dueño del `Child` y lo cosecha cuando el greeter
/// termina.
///
/// `lock_for`: `None` ⇒ greeter de login (modo DM). `Some(usuario)` ⇒ greeter
/// en modo **lock** (`--lock`), con el usuario fijo (dueño de la sesión) pasado
/// por `MIRADA_LOCK_USER` — pide su contraseña para desbloquear.
///
/// Devuelve el `stdin` del greeter: el compositor le empuja por ahí la
/// disposición de monitores (qué monitor tiene el ratón) para que la tarjeta
/// viaje al monitor activo. Ver [`crate::estado::App::greeter_stdin`].
pub(crate) fn spawn_greeter<S>(
    lock_for: Option<&str>,
    send: S,
) -> std::io::Result<std::process::ChildStdin>
where
    S: Fn(ShellAction) + Send + 'static,
{
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let mut command = Command::new(greeter_bin());
    command
        .envs(THEME_ENV.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    // Modo lock: bandera + usuario fijo (el shell-lock no deja teclear el nombre).
    if let Some(user) = lock_for {
        command.arg("--lock").env("MIRADA_LOCK_USER", user);
    }
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("stdout pedido con Stdio::piped");
    let stdin = child.stdin.take().expect("stdin pedido con Stdio::piped");
    let papel = if lock_for.is_some() { "lock" } else { "login" };
    println!("mirada-compositor · greeter ({papel}) lanzado (pid {}).", child.id());
    if std::env::var_os("LLIMPHI_TIMING").is_some() {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        eprintln!("[llimphi-timing] mirada:greeter-spawn epoch_ms={ms}");
    }

    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            match ShellAction::from_line(&line) {
                Some(action) => {
                    println!("mirada-compositor · acción del shell recibida del greeter.");
                    send(action);
                }
                None => println!("greeter · {line}"),
            }
        }
        match child.wait() {
            Ok(status) => println!("mirada-compositor · el greeter terminó ({status})."),
            Err(e) => dlog!("mirada-compositor · wait(greeter): {e}"),
        }
    });
    Ok(stdin)
}

#[cfg(test)]
mod tests_magnify {
    use super::magnify_origin;

    #[test]
    fn factor_uno_es_identidad() {
        let (o, s) = magnify_origin((1920, 1080), (960.0, 540.0), 1.0);
        assert_eq!((o.x, o.y), (0, 0));
        assert_eq!(s, 1.0);
    }

    #[test]
    fn salida_degenerada_es_identidad() {
        let (o, s) = magnify_origin((0, 0), (10.0, 10.0), 2.0);
        assert_eq!((o.x, o.y), (0, 0));
        assert_eq!(s, 1.0);
    }

    #[test]
    fn foco_centrado_fija_el_centro() {
        // Puntero en el centro: el reescalado se ancla en el centro y conserva
        // el factor pedido.
        let (o, s) = magnify_origin((1000, 800), (500.0, 400.0), 2.0);
        assert_eq!((o.x, o.y), (500, 400));
        assert_eq!(s, 2.0);
    }

    #[test]
    fn foco_en_esquina_se_acota_al_borde() {
        // Puntero en (0,0): el origen se pega a la esquina superior-izquierda,
        // mostrando la región [0, w/z] — sin paneár fuera de la salida.
        let (o, _) = magnify_origin((1000, 800), (0.0, 0.0), 2.0);
        assert_eq!((o.x, o.y), (0, 0));
    }

    #[test]
    fn foco_en_esquina_opuesta_se_acota_al_otro_borde() {
        let (o, _) = magnify_origin((1000, 800), (1000.0, 800.0), 2.0);
        assert_eq!((o.x, o.y), (1000, 800));
    }

    #[test]
    fn el_origen_nunca_se_sale_de_la_salida() {
        // Para cualquier foco (incluso fuera de rango) y factor, origin ∈ [0,len].
        for &f in &[-50.0, 0.0, 123.0, 500.0, 999.0, 1200.0] {
            for &z in &[1.5_f32, 2.0, 4.0, 8.0] {
                let (o, _) = magnify_origin((1000, 800), (f, f.min(800.0)), z);
                assert!((0..=1000).contains(&o.x), "x={} z={}", o.x, z);
                assert!((0..=800).contains(&o.y), "y={} z={}", o.y, z);
            }
        }
    }
}

#[cfg(test)]
mod tests_titlebar {
    use super::*;
    use mirada_aware::{AwareItem, AwareSide};
    use mirada_brain::{TitlebarAction, TitlebarItem, TitlebarLayout};

    fn es_accion(cell: &TbCell, want: TitlebarAction) -> bool {
        cell.action() == Some(&want)
    }

    #[test]
    fn layout_clasico_pone_cerrar_a_la_derecha_del_todo() {
        // Default: derecha = [min, max, close]. Contenido en cx=100, cw=400.
        let layout = TitlebarLayout::default();
        let cells = titlebar_cells_for(&layout, &[], 100, 400);
        assert_eq!(cells.len(), 3);
        // El último (cerrar) queda pegado al borde derecho (cx+cw-W).
        let der = cells.iter().max_by_key(|(x, _)| *x).unwrap();
        assert_eq!(der.0, 100 + 400 - TB_BTN_W);
        assert!(es_accion(&der.1, TitlebarAction::Close));
    }

    #[test]
    fn grupos_izquierda_y_derecha_no_se_pisan() {
        let layout = TitlebarLayout {
            left: vec![TitlebarItem::button(TitlebarAction::Menu)],
            right: vec![
                TitlebarItem::button(TitlebarAction::Minimize),
                TitlebarItem::button(TitlebarAction::Maximize),
            ],
            ..Default::default()
        };
        let cells = titlebar_cells_for(&layout, &[], 0, 400);
        assert_eq!(cells.len(), 3);
        assert_eq!(cells[0].0, 0);
        assert!(es_accion(&cells[0].1, TitlebarAction::Menu));
        for (x, _) in &cells {
            assert!(*x >= 0 && *x + TB_BTN_W <= 400);
        }
    }

    #[test]
    fn ventana_angosta_conserva_cerrar() {
        let layout = TitlebarLayout::default();
        let cells = titlebar_cells_for(&layout, &[], 0, TB_BTN_W + 6);
        assert_eq!(cells.len(), 1);
        assert!(es_accion(&cells[0].1, TitlebarAction::Close));
    }

    #[test]
    fn las_contribuciones_de_app_entran_entre_titulo_y_botones_de_sistema() {
        // App aporta un botón a la derecha; debe quedar a la IZQUIERDA de los de
        // sistema (cerrar sigue pegado al borde), y aparecer como TbCell::App.
        let layout = TitlebarLayout::default(); // right = [min, max, close]
        let contribs = vec![AwareItem { id: "run".into(), glyph: "▶".into(), label: "Correr".into(), side: AwareSide::Right }];
        let cells = titlebar_cells_for(&layout, &contribs, 0, 400);
        assert_eq!(cells.len(), 4, "3 de sistema + 1 aportado");
        // Cerrar sigue siendo el más a la derecha.
        let der = cells.iter().max_by_key(|(x, _)| *x).unwrap();
        assert!(es_accion(&der.1, TitlebarAction::Close));
        // El aportado existe y está a la izquierda de los 3 de sistema.
        let app = cells.iter().find(|(_, c)| matches!(c, TbCell::App { item_id, .. } if *item_id == "run")).unwrap();
        assert!(app.0 < cells.iter().filter(|(_, c)| matches!(c, TbCell::Sys(_))).map(|(x, _)| *x).min().unwrap());
    }
}
