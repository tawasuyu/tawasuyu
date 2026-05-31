use super::*;

/// FASE 58 :: el launcher como duenio del teclado. Devuelve `true` si el
/// `mando` se atendio dentro del overlay y no debe pasar al despacho normal.
/// `Mando::ToggleLauncher` se atiende siempre aqui —abre o cierra—; el resto
/// solo se interpreta si el overlay esta abierto.
///
/// Atajos mientras el launcher esta abierto:
///   * `Alt+J` / `Alt+K` mueven la seleccion abajo y arriba (cicla).
///   * `Alt+Enter` lanza la app seleccionada y cierra el overlay.
///   * `Alt+Q` cierra el overlay sin lanzar nada.
///   * `Alt+P` (toggle) tambien cierra.
///
/// Cualquier otro mando se descarta: el escritorio NO debe mutar por debajo
/// del overlay mientras el operador esta eligiendo.
pub(crate) fn launcher_intercepta(mando: Mando) -> bool {
    let Some(escritorio) = ESCRITORIO.get() else {
        return false;
    };
    let mut escritorio = escritorio.lock();

    if matches!(mando, Mando::ToggleLauncher) {
        if escritorio.launcher_abierto {
            cerrar_launcher(&mut escritorio);
        } else {
            abrir_launcher(&mut escritorio);
        }
        recomponer(&mut escritorio);
        return true;
    }

    if !escritorio.launcher_abierto {
        return false;
    }

    match mando {
        Mando::FocoSiguiente => {
            let n = escritorio.launcher_filtrado.len();
            if n > 0 {
                escritorio.launcher_seleccion = (escritorio.launcher_seleccion + 1) % n;
                ajustar_scroll_launcher(&mut escritorio);
            }
        }
        Mando::FocoAnterior => {
            let n = escritorio.launcher_filtrado.len();
            if n > 0 {
                escritorio.launcher_seleccion = (escritorio.launcher_seleccion + n - 1) % n;
                ajustar_scroll_launcher(&mut escritorio);
            }
        }
        Mando::Promover => {
            if let Some(&idx_real) = escritorio
                .launcher_filtrado
                .get(escritorio.launcher_seleccion)
            {
                if let Some(cola) = PARTOS_POR_INDICE.get() {
                    cola.lock().push(idx_real);
                }
            }
            cerrar_launcher(&mut escritorio);
        }
        Mando::Cerrar => {
            cerrar_launcher(&mut escritorio);
        }
        Mando::TextoLauncher(byte) => {
            // FASE 58 v3 :: edicion en vivo de la query. Backspace borra el
            // ultimo caracter; cualquier otro byte se trata como ASCII y se
            // agrega si cabe en el techo de longitud. Tras tocar la query,
            // refiltrar el catalogo y reanclar la seleccion al inicio para
            // que el primer match siempre quede visible.
            const BACKSPACE: u8 = 0x08;
            if byte == BACKSPACE {
                escritorio.launcher_query.pop();
            } else if escritorio.launcher_query.len() < QUERY_MAX_LEN {
                escritorio.launcher_query.push(byte as char);
            }
            refiltrar_launcher(&mut escritorio);
        }
        Mando::LanzarFila(visible) => {
            // FASE 58 v8 :: el operador pulso `Alt+<digito>` sobre la fila
            // visible `visible` (0..=8 = filas 1..9 del launcher). El indice
            // absoluto en el filtrado es `scroll + visible`. Si la fila no
            // existe (filtrado mas corto que el visible), se descarta en
            // silencio —Alt+5 sobre un filtrado de 3 apps no hace nada en
            // lugar de explotar—.
            let idx_absoluto = escritorio.launcher_scroll + visible;
            if let Some(&idx_real) = escritorio.launcher_filtrado.get(idx_absoluto) {
                if let Some(cola) = PARTOS_POR_INDICE.get() {
                    cola.lock().push(idx_real);
                }
                cerrar_launcher(&mut escritorio);
            }
        }
        // Cualquier otro mando se descarta — el launcher se queda con el
        // foco del teclado hasta cerrarse.
        _ => {}
    }

    recomponer(&mut escritorio);
    true
}

/// FASE 58 v3 :: abre el overlay, sembrando el filtrado con el catalogo
/// entero y reseteando la query y la seleccion. Sincroniza el mirror
/// atomico `LAUNCHER_ABIERTO` para que el IRQ del teclado vea el cambio.
pub(crate) fn abrir_launcher(escritorio: &mut Escritorio) {
    escritorio.launcher_abierto = true;
    escritorio.launcher_query.clear();
    escritorio.launcher_seleccion = 0;
    escritorio.launcher_scroll = 0;
    refiltrar_launcher(escritorio);
    LAUNCHER_ABIERTO.store(true, Ordering::Relaxed);
}

/// FASE 58 v3 :: cierra el overlay y libera la query. El filtrado se vacia
/// para que el siguiente `abrir_launcher` arranque desde cero —no quedan
/// indices viejos colgando si el catalogo crecio entre aperturas—.
pub(crate) fn cerrar_launcher(escritorio: &mut Escritorio) {
    escritorio.launcher_abierto = false;
    escritorio.launcher_query.clear();
    escritorio.launcher_filtrado.clear();
    escritorio.launcher_mascaras.clear();
    escritorio.launcher_seleccion = 0;
    escritorio.launcher_scroll = 0;
    LAUNCHER_ABIERTO.store(false, Ordering::Relaxed);
}

/// FASE 58 v5 :: recalcula `launcher_filtrado` contra la query vigente con
/// match jerarquico — los nombres se ordenan por *calidad* del match, no por
/// el orden del manifiesto—. Tres niveles, mejor primero:
///
///   3. prefijo  — el nombre empieza con la query (case-insensitive).
///   2. substring — la query aparece contigua dentro del nombre.
///   1. subsecuencia — las letras de la query aparecen en orden, no
///      necesariamente pegadas (estilo fzf/Spotlight: "plm" matchea "pluma").
///
/// Dentro de cada nivel, gana el que tiene la primera letra emparejada mas
/// cerca del inicio; en empate, el orden original del manifiesto. La seleccion
/// previa se preserva si la app sigue lanzable —backspace ya no tira el cursor
/// al primer item, como pasaba en v3—.
///
/// FASE 58 v6 :: en paralelo a `launcher_filtrado`, se llena
/// `launcher_mascaras` con la mascara de chars matcheados por nombre —el
/// pintado del overlay las usa para resaltar las letras del match (Spotlight
/// classic).
pub(crate) fn refiltrar_launcher(escritorio: &mut Escritorio) {
    // Si habia algo seleccionado, anclamos su indice de catalogo para
    // intentar recolocarlo tras refiltrar.
    let sel_previa = escritorio
        .launcher_filtrado
        .get(escritorio.launcher_seleccion)
        .copied();

    escritorio.launcher_filtrado.clear();
    escritorio.launcher_mascaras.clear();
    let query = &escritorio.launcher_query;
    if query.is_empty() {
        // Sin query: todo el catalogo es valido, en su orden original.
        // Mascara cero = ningun char marcado (no hay nada que resaltar).
        for i in 0..escritorio.catalogo.len() {
            escritorio.launcher_filtrado.push(i);
            escritorio.launcher_mascaras.push(0);
        }
    } else {
        // Reunimos (nivel, primer_match, mascara, indice_catalogo) para los
        // que matcheen — Vec temporal porque el catalogo es chico (12 hoy)
        // y la refiltracion ocurre una vez por keystroke, no en el camino
        // caliente del compositor.
        let mut ranking: Vec<(u8, usize, u64, usize)> = Vec::new();
        for (i, nombre) in escritorio.catalogo.iter().enumerate() {
            if let Some((tier, mask)) = evaluar_match(nombre, query) {
                let primer = mask.trailing_zeros() as usize;
                ranking.push((tier, primer, mask, i));
            }
        }
        // Orden: nivel desc, primer_match asc, indice_catalogo asc.
        ranking.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then(a.1.cmp(&b.1))
                .then(a.3.cmp(&b.3))
        });
        for (_, _, mask, idx) in ranking {
            escritorio.launcher_filtrado.push(idx);
            escritorio.launcher_mascaras.push(mask);
        }
    }

    // Recolocar la seleccion donde quedo la app previa si sigue en el listado;
    // si desaparecio (o no habia previa), volver a la cabeza.
    escritorio.launcher_seleccion = sel_previa
        .and_then(|prev| {
            escritorio
                .launcher_filtrado
                .iter()
                .position(|&i| i == prev)
        })
        .unwrap_or(0);
    // FASE 58 v7 :: tras refiltrar, el viewport se reposiciona para que la
    // seleccion vigente sea visible — si la lista se acorto o la sel. se
    // movio, el scroll viejo puede haber quedado fuera del rango valido.
    ajustar_scroll_launcher(escritorio);
}

/// FASE 58 v7 :: ajusta `launcher_scroll` para mantener `launcher_seleccion`
/// dentro del viewport visible `[scroll, scroll + PICKER_MAX_FILAS)`. Si el
/// catalogo filtrado cabe entero, el scroll queda en 0; si la seleccion
/// quedo por encima del scroll, lo arrastramos hasta ponerla en la cabeza
/// del viewport; si quedo por debajo, lo empujamos hasta dejarla en la
/// cola. Sin animacion: el viewport salta lo mas corto posible.
pub(crate) fn ajustar_scroll_launcher(escritorio: &mut Escritorio) {
    let total = escritorio.launcher_filtrado.len();
    let sel = escritorio.launcher_seleccion;
    if total <= PICKER_MAX_FILAS {
        escritorio.launcher_scroll = 0;
        return;
    }
    let mut scroll = escritorio.launcher_scroll;
    // Cota superior: el ultimo scroll que aun deja PICKER_MAX_FILAS filas
    // visibles —no tiene sentido scrollear mas alla del final del listado—.
    let scroll_max = total - PICKER_MAX_FILAS;
    if scroll > scroll_max {
        scroll = scroll_max;
    }
    // La seleccion vive arriba del viewport: arrastrar el viewport hacia ella.
    if sel < scroll {
        scroll = sel;
    }
    // La seleccion vive bajo el viewport: empujarlo hasta dejarla en la cola.
    if sel >= scroll + PICKER_MAX_FILAS {
        scroll = sel + 1 - PICKER_MAX_FILAS;
    }
    escritorio.launcher_scroll = scroll;
}

/// FASE 58 v5+v6 :: evalua el match de `aguja` contra `pajar` y devuelve
/// `(nivel, mascara)` o `None` si no hay match ni siquiera como subsecuencia.
/// `nivel` clasifica la calidad del match (3 = prefijo, 2 = substring, 1 =
/// subsecuencia). `mascara` tiene el bit `i` a 1 si el caracter `i` de `pajar`
/// formo parte del match —el llamante lo usa para resaltar las letras
/// matcheadas en el overlay (Spotlight-classic).
///
/// Para nivel 3 (prefijo) los bits 0..aguja.len() estan a 1; para nivel 2
/// (substring) los bits inicio..inicio+aguja.len(); para nivel 1
/// (subsecuencia) los bits dispersos correspondientes al greedy de izquierda
/// a derecha. Caracteres mas alla del bit 63 nunca se marcan (los nombres
/// del manifiesto son cortos —los mas largos llevan 9 chars hoy—).
///
/// Case-insensitive sobre ASCII; bytes no-ASCII se comparan crudos —pueden
/// no matchear pero no causan panico—.
pub(crate) fn evaluar_match(pajar: &str, aguja: &str) -> Option<(u8, u64)> {
    let pajar = pajar.as_bytes();
    let aguja = aguja.as_bytes();
    if aguja.is_empty() {
        return Some((3, 0));
    }
    let eq = |a: u8, b: u8| a.to_ascii_lowercase() == b.to_ascii_lowercase();
    // Helper: mascara contigua de `n` bits arrancando en `inicio` (chars
    // mas alla del bit 63 se truncan silenciosamente). Construirla a mano
    // evita los casos de borde de `(1 << n) - 1` cuando n = 64.
    let mascara_contigua = |inicio: usize, n: usize| -> u64 {
        let mut m: u64 = 0;
        for k in 0..n {
            let bit = inicio + k;
            if bit < 64 {
                m |= 1u64 << bit;
            }
        }
        m
    };

    // Nivel 3 — prefijo.
    if pajar.len() >= aguja.len()
        && pajar[..aguja.len()]
            .iter()
            .zip(aguja.iter())
            .all(|(a, b)| eq(*a, *b))
    {
        return Some((3, mascara_contigua(0, aguja.len())));
    }

    // Nivel 2 — substring contiguo.
    if pajar.len() >= aguja.len() {
        for inicio in 1..=(pajar.len() - aguja.len()) {
            let ventana = &pajar[inicio..inicio + aguja.len()];
            if ventana
                .iter()
                .zip(aguja.iter())
                .all(|(a, b)| eq(*a, *b))
            {
                return Some((2, mascara_contigua(inicio, aguja.len())));
            }
        }
    }

    // Nivel 1 — subsecuencia (cada caracter en orden, no necesariamente
    // contiguo). Recorremos pajar de izquierda a derecha consumiendo aguja
    // a medida que casa; si terminamos aguja entera, hay match. Marcamos
    // cada posicion casada en la mascara.
    let mut iter = pajar.iter().enumerate();
    let mut mascara: u64 = 0;
    'siguiente_letra: for &ch_a in aguja {
        for (idx, &ch_p) in iter.by_ref() {
            if eq(ch_p, ch_a) {
                if idx < 64 {
                    mascara |= 1u64 << idx;
                }
                continue 'siguiente_letra;
            }
        }
        return None;
    }
    Some((1, mascara))
}
