//! El modelo de capacidades de los plugins — espejo host-side del bitfield
//! `Permisos` del kernel wawa. Cada capacidad gatea una importación del host:
//! si el bit no está concedido, la función no se registra en el `Linker` y un
//! módulo que la importe ni instancia (frontera física, no chequeada en runtime).

/// Bitfield de capacidades concedidas a un plugin.
pub type CapsPlugin = u32;

/// Decidir la geometría de las ventanas teseladas (plugin de layout).
pub const CAP_LAYOUT: CapsPlugin = 1 << 0;
/// Lanzar programas (`BrainCommand::Spawn`).
pub const CAP_SPAWN: CapsPlugin = 1 << 1;
/// Cerrar/matar ventanas (`Close`/`Kill`).
pub const CAP_WINDOW_CONTROL: CapsPlugin = 1 << 2;
/// Registrar atajos globales (`GrabKeys`).
pub const CAP_KEYS: CapsPlugin = 1 << 3;
/// Fijar decoración/cursor (`SetDecorations`/`SetCursor`).
pub const CAP_DECOR: CapsPlugin = 1 << 4;

/// Traduce un nombre de capacidad del manifest a su bit.
pub fn parse_cap(name: &str) -> Option<CapsPlugin> {
    Some(match name {
        "layout" => CAP_LAYOUT,
        "spawn" => CAP_SPAWN,
        "window_control" => CAP_WINDOW_CONTROL,
        "keys" => CAP_KEYS,
        "decor" => CAP_DECOR,
        _ => return None,
    })
}

/// Nombre legible de un bit, para diagnósticos.
pub fn cap_name(bit: CapsPlugin) -> &'static str {
    match bit {
        CAP_LAYOUT => "layout",
        CAP_SPAWN => "spawn",
        CAP_WINDOW_CONTROL => "window_control",
        CAP_KEYS => "keys",
        CAP_DECOR => "decor",
        _ => "?",
    }
}

/// Lista legible de las capacidades de un bitfield (para errores).
pub fn caps_list(caps: CapsPlugin) -> String {
    let mut out = Vec::new();
    for bit in [CAP_LAYOUT, CAP_SPAWN, CAP_WINDOW_CONTROL, CAP_KEYS, CAP_DECOR] {
        if caps & bit != 0 {
            out.push(cap_name(bit));
        }
    }
    if out.is_empty() {
        "{}".to_string()
    } else {
        format!("{{{}}}", out.join(", "))
    }
}

/// La capacidad que gatea una importación `mirada_host::<name>`.
///
/// - `Some(0)` → importación sin capacidad (siempre permitida, p. ej. `host_log`).
/// - `Some(bit)` → requiere ese bit concedido.
/// - `None` → importación desconocida en el namespace `mirada_host` → rechazo.
pub fn cap_for_import(name: &str) -> Option<CapsPlugin> {
    Some(match name {
        "host_log" => 0,
        "host_emit_spawn" => CAP_SPAWN,
        "host_emit_close" | "host_emit_kill" => CAP_WINDOW_CONTROL,
        "host_emit_keys" => CAP_KEYS,
        "host_emit_decor" | "host_emit_cursor" => CAP_DECOR,
        _ => return None,
    })
}
