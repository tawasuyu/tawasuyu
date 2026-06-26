//! `fus` — *fast user switching*: el roster de sesiones hosteadas.
//!
//! El compositor puede hostear **N sesiones concurrentes** (cada una de un
//! usuario, con sus apps rebajadas a su uid) y mostrar una a la vez. Este
//! módulo es la **política pura** de ese roster: alta, baja, cuál está activa y
//! a cuál saltar. Es agnóstico de `smithay` y del contenido de cada sesión —
//! parametrizado por `S` (en el compositor, `S = Session` con su `UserInfo` y
//! entorno). Vive en el Cerebro por la misma razón que el resto de la política:
//! es determinista y testeable sin levantar una pantalla.
//!
//! **Ids estables, no índices.** Cada sesión recibe un [`SessionId`] que **no
//! cambia** al dar de baja otra: las ventanas se etiquetan con el id de su
//! sesión (no con un índice que se correría al cerrar una sesión previa). Es la
//! diferencia que vuelve seguro el multiplexado de ventanas por sesión.
//!
//! **N≤1 = comportamiento de siempre.** El roster nace vacío; el primer `add`
//! (el traspaso del greeter) deja una sola sesión activa, idéntico al camino
//! single-session anterior. El multiplexado real sólo entra con ≥2 sesiones.

/// Identificador estable de una sesión hosteada. Único dentro de un roster y
/// **monótono**: no se reusa al dar de baja una sesión, así una ventana
/// etiquetada con un id viejo nunca se confunde con una sesión nueva.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct SessionId(pub u32);

/// El roster de sesiones del compositor. `S` es lo que el host adjunta a cada
/// sesión (usuario + entorno). Mantiene el orden de alta y cuál está activa.
#[derive(Debug, Default)]
pub struct SessionRoster<S> {
    sessions: Vec<(SessionId, S)>,
    active: Option<SessionId>,
    next_id: u32,
}

impl<S> SessionRoster<S> {
    /// Roster vacío (modo greeter de arranque: ninguna sesión todavía).
    pub fn new() -> Self {
        SessionRoster {
            sessions: Vec::new(),
            active: None,
            next_id: 0,
        }
    }

    /// Da de alta una sesión y la **activa** (la recién llegada pasa al frente,
    /// como tras un login). Devuelve su id estable.
    pub fn add(&mut self, session: S) -> SessionId {
        let id = SessionId(self.next_id);
        self.next_id += 1;
        self.sessions.push((id, session));
        self.active = Some(id);
        id
    }

    /// Da de baja la sesión `id` (logout). Si era la activa, el foco pasa a la
    /// **última** sesión restante (la más reciente) o a `None` si no queda
    /// ninguna. Devuelve la `S` removida si existía.
    pub fn remove(&mut self, id: SessionId) -> Option<S> {
        let pos = self.sessions.iter().position(|(sid, _)| *sid == id)?;
        let (_, s) = self.sessions.remove(pos);
        if self.active == Some(id) {
            self.active = self.sessions.last().map(|(sid, _)| *sid);
        }
        Some(s)
    }

    /// Salta el foco a la sesión `id`. `true` si existía (y ahora es la activa),
    /// `false` si no hay tal sesión (el foco no cambia).
    pub fn switch_to(&mut self, id: SessionId) -> bool {
        if self.sessions.iter().any(|(sid, _)| *sid == id) {
            self.active = Some(id);
            true
        } else {
            false
        }
    }

    /// Id de la sesión activa, o `None` en modo greeter (ninguna).
    pub fn active_id(&self) -> Option<SessionId> {
        self.active
    }

    /// La sesión activa (su `S`), o `None`.
    pub fn active(&self) -> Option<&S> {
        let id = self.active?;
        self.get(id)
    }

    /// La sesión activa mutable.
    pub fn active_mut(&mut self) -> Option<&mut S> {
        let id = self.active?;
        self.get_mut(id)
    }

    /// La sesión `id`, si existe.
    pub fn get(&self, id: SessionId) -> Option<&S> {
        self.sessions
            .iter()
            .find(|(sid, _)| *sid == id)
            .map(|(_, s)| s)
    }

    /// La sesión `id` mutable.
    pub fn get_mut(&mut self, id: SessionId) -> Option<&mut S> {
        self.sessions
            .iter_mut()
            .find(|(sid, _)| *sid == id)
            .map(|(_, s)| s)
    }

    /// `true` si `id` es la sesión activa. La regla de visibilidad del
    /// compositor: una ventana se muestra si su sesión es la activa.
    pub fn is_active(&self, id: SessionId) -> bool {
        self.active == Some(id)
    }

    /// Cuántas sesiones hay hosteadas. `len() <= 1` ⇒ sin multiplexado (camino
    /// single-session, byte-idéntico al de siempre).
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// `true` si no hay ninguna sesión (modo greeter de arranque).
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Los ids en orden de alta — para que el lock pinte el selector de «cambiar
    /// usuario» y para iterar sobre todas las sesiones.
    pub fn ids(&self) -> impl Iterator<Item = SessionId> + '_ {
        self.sessions.iter().map(|(id, _)| *id)
    }

    /// Itera `(id, &S)` en orden de alta.
    pub fn iter(&self) -> impl Iterator<Item = (SessionId, &S)> + '_ {
        self.sessions.iter().map(|(id, s)| (*id, s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vacio_es_modo_greeter() {
        let r: SessionRoster<&str> = SessionRoster::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
        assert_eq!(r.active_id(), None);
        assert!(r.active().is_none());
    }

    #[test]
    fn primer_add_activa_y_es_single_session() {
        let mut r = SessionRoster::new();
        let a = r.add("ana");
        assert_eq!(r.len(), 1);
        assert_eq!(r.active_id(), Some(a));
        assert_eq!(r.active(), Some(&"ana"));
        assert!(r.is_active(a));
    }

    #[test]
    fn add_sucesivo_activa_al_nuevo_con_ids_distintos() {
        let mut r = SessionRoster::new();
        let a = r.add("ana");
        let b = r.add("beto");
        assert_ne!(a, b);
        // El recién llegado queda activo (como tras un login).
        assert_eq!(r.active_id(), Some(b));
        assert!(r.is_active(b));
        assert!(!r.is_active(a));
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn switch_to_existente_y_inexistente() {
        let mut r = SessionRoster::new();
        let a = r.add("ana");
        let b = r.add("beto");
        assert!(r.switch_to(a));
        assert!(r.is_active(a));
        assert!(!r.is_active(b));
        // Saltar a un id que no existe no cambia el foco.
        assert!(!r.switch_to(SessionId(999)));
        assert!(r.is_active(a));
    }

    #[test]
    fn remove_activa_pasa_el_foco_a_la_ultima() {
        let mut r = SessionRoster::new();
        let a = r.add("ana");
        let b = r.add("beto");
        let c = r.add("caro");
        assert!(r.is_active(c));
        // Cerrar la activa: el foco cae en la última restante (beto).
        assert_eq!(r.remove(c), Some("caro"));
        assert_eq!(r.active_id(), Some(b));
        // Cerrar una no-activa no toca el foco.
        assert_eq!(r.remove(a), Some("ana"));
        assert_eq!(r.active_id(), Some(b));
        // Cerrar la última deja el roster vacío (vuelta a greeter).
        assert_eq!(r.remove(b), Some("beto"));
        assert!(r.is_empty());
        assert_eq!(r.active_id(), None);
    }

    #[test]
    fn ids_no_se_reusan_tras_baja() {
        let mut r = SessionRoster::new();
        let a = r.add("ana");
        r.remove(a);
        let b = r.add("beto");
        // El nuevo id es monótono: jamás colisiona con el de una sesión muerta.
        assert_ne!(a, b);
        assert_eq!(b, SessionId(1));
    }

    #[test]
    fn get_y_active_mut() {
        let mut r = SessionRoster::new();
        let a = r.add(String::from("ana"));
        r.get_mut(a).unwrap().push_str("-x");
        assert_eq!(r.get(a).map(String::as_str), Some("ana-x"));
        r.active_mut().unwrap().push_str("-y");
        assert_eq!(r.active().map(String::as_str), Some("ana-x-y"));
    }

    #[test]
    fn ids_en_orden_de_alta() {
        let mut r = SessionRoster::new();
        let a = r.add("ana");
        let b = r.add("beto");
        let got: Vec<_> = r.ids().collect();
        assert_eq!(got, vec![a, b]);
    }
}
