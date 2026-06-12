// Funciones utilitarias del compositor.
use crate::*;
use smithay::input::keyboard::{xkb, ModifiersState, Keysym};
use smithay::input::pointer::CursorImageSurfaceData;
use smithay::wayland::compositor::{with_states, with_surface_tree_downward, SurfaceAttributes, TraversalAction};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::SERIAL_COUNTER;
use auth_core::{SessionTicket, UserInfo};

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
    eprintln!("mirada-compositor · no pude ceder a «{cmd}»: {err}");
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

/// El alto efectivo de la barra de título de `w`: `0` para el shell y las
/// ventanas a pantalla completa (no llevan), el `titlebar_height` configurado
/// para el resto. Acotado a `>= 0`.
pub(crate) fn titlebar_for(w: &ManagedWindow, titlebar_height: i32) -> i32 {
    if w.is_shell || w.fullscreen {
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
    match with_renderer_surface_state(&w.surface, |s| s.surface_size()) {
        Some(Some(size)) => {
            let dx = ((w.size.0 - size.w) / 2).max(0);
            let dy = ((content_h - size.h) / 2).max(0);
            (w.loc.0 + dx, content_top + dy)
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

pub(crate) fn spawn_command(cmd: &str, as_user: Option<&UserInfo>, session_env: &[(String, String)]) {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return;
    }
    let mut command = std::process::Command::new("sh");
    command.arg("-c").arg(cmd).envs(THEME_ENV.iter().copied());
    // Entorno de sesión (runtime dir del usuario, WAYLAND_DISPLAY absoluto,
    // bus D-Bus) — vacío para el greeter, poblado tras el traspaso.
    for (k, v) in session_env {
        command.env(k, v);
    }
    if let Some(user) = as_user {
        if nix::unistd::geteuid().is_root() {
            apply_user(&mut command, user);
        }
    }
    match command.spawn() {
        Ok(child) => println!("mirada-compositor · lanzado (pid {}): {cmd}", child.id()),
        Err(e) => eprintln!("mirada-compositor · no pude lanzar «{cmd}»: {e}"),
    }
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

/// Nombre o ruta del binario del greeter. `MIRADA_GREETER_BIN` lo
/// sobreescribe — cómodo en desarrollo para apuntar a `target/…`.
pub(crate) fn greeter_bin() -> String {
    std::env::var("MIRADA_GREETER_BIN").unwrap_or_else(|_| "mirada-greeter".to_string())
}

/// Lanza `mirada-greeter` como proceso hijo, en modo DM, con el stdout
/// capturado. Un hilo lee sus líneas: la que sea un [`SessionTicket`] se
/// entrega por `send` (el bucle de eventos hará el traspaso); el resto
/// del stdout se reenvía a la consola con el prefijo `greeter ·`. El
/// hilo es dueño del `Child` y lo cosecha cuando el greeter termina.
pub(crate) fn spawn_greeter<S>(send: S) -> std::io::Result<()>
where
    S: Fn(SessionTicket) + Send + 'static,
{
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let mut child = Command::new(greeter_bin())
        .envs(THEME_ENV.iter().copied())
        .stdout(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().expect("stdout pedido con Stdio::piped");
    println!("mirada-compositor · greeter lanzado (pid {}).", child.id());

    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            match SessionTicket::from_line(&line) {
                Some(ticket) => {
                    println!("mirada-compositor · tiquet de sesión recibido del greeter.");
                    send(ticket);
                }
                None => println!("greeter · {line}"),
            }
        }
        match child.wait() {
            Ok(status) => println!("mirada-compositor · el greeter terminó ({status})."),
            Err(e) => eprintln!("mirada-compositor · wait(greeter): {e}"),
        }
    });
    Ok(())
}
