//! Lógica del modo Sistema: ingesta de barridos `/proc`, árbol padre/hijo,
//! filtro, scroll y ordenación de la lista de procesos.

use std::collections::{HashMap, HashSet};

use llimphi_ui::Handle;

use super::engine::wawa_census;
use super::modelo::{Msg, Model, Sort, SysProc, Tab, GRAPH_LEN, SYS_ROWS};
use super::procfs::Scan;

// ---------------------------------------------------------------------------
// Una fila tal como se va a pintar.
// ---------------------------------------------------------------------------

/// Una fila tal como se va a pintar: índice en `model.system`, profundidad en
/// el árbol y si tiene hijos (para el triángulo de colapso).
#[derive(Clone, Copy)]
pub(crate) struct RenderRow {
    pub(crate) idx: usize,
    pub(crate) depth: u16,
    pub(crate) has_kids: bool,
}

// ---------------------------------------------------------------------------
// Helpers de historial.
// ---------------------------------------------------------------------------

/// Empuja una muestra al historial, recortando a `GRAPH_LEN`.
pub(crate) fn push_capped(buf: &mut std::collections::VecDeque<f32>, v: f32) {
    if buf.len() == GRAPH_LEN {
        buf.pop_front();
    }
    buf.push_back(v);
}

// ---------------------------------------------------------------------------
// Ingesta de un barrido de /proc.
// ---------------------------------------------------------------------------

/// Toma un barrido crudo de `/proc` y deriva %CPU/%MEM contra la lectura
/// previa (guardada en el Model). Deja `model.system` ordenado.
/// Empuja una muestra al historial, recortando a `GRAPH_LEN`.
pub(crate) fn ingest_system(model: &mut Model, scan: Scan) {
    let dtotal = scan.total_jiffies.saturating_sub(model.prev_total).max(1) as f32;
    let ncpu = scan.ncpu.max(1) as f32;
    let mem_total = scan.mem_total_kb.max(1) as f32;

    let mut next_prev = HashMap::with_capacity(scan.procs.len());
    let mut out = Vec::with_capacity(scan.procs.len());
    for p in &scan.procs {
        let dproc = p
            .cpu_jiffies
            .saturating_sub(model.prev_proc.get(&p.pid).copied().unwrap_or(p.cpu_jiffies))
            as f32;
        // delta_proc / delta_total_de_una_cpu = delta_proc / (dtotal/ncpu).
        let cpu_pct = (dproc / (dtotal / ncpu)).clamp(0.0, 100.0 * ncpu);
        next_prev.insert(p.pid, p.cpu_jiffies);
        let uptime_secs = if scan.clk_tck > 0 {
            (scan.uptime_secs - p.starttime_ticks as f64 / scan.clk_tck as f64).max(0.0) as u64
        } else {
            0
        };
        out.push(SysProc {
            pid: p.pid,
            ppid: p.ppid,
            name: p.name.clone(),
            state: p.state,
            cpu_pct,
            mem_pct: (p.rss_kb as f32 / mem_total) * 100.0,
            rss_kb: p.rss_kb,
            threads: p.threads,
            uid: p.uid,
            uptime_secs,
            cmd: p.cmd.clone(),
        });
    }

    // %uso por core: delta(busy)/delta(total) contra la lectura previa.
    if model.core_hist.len() != scan.cores.len() {
        model.core_hist = vec![std::collections::VecDeque::new(); scan.cores.len()];
        model.prev_core = vec![(0, 0); scan.cores.len()];
    }
    for (i, &(_id, total, idle)) in scan.cores.iter().enumerate() {
        let (ptotal, pidle) = model.prev_core[i];
        let dtot = total.saturating_sub(ptotal) as f32;
        let didle = idle.saturating_sub(pidle) as f32;
        let usage = if dtot > 0.0 {
            ((dtot - didle) / dtot).clamp(0.0, 1.0) * 100.0
        } else {
            0.0
        };
        push_capped(&mut model.core_hist[i], usage);
    }
    model.core_ids = scan.cores.iter().map(|&(id, _, _)| id).collect();
    model.prev_core = scan.cores.iter().map(|&(_, t, i)| (t, i)).collect();

    // % de memoria usada para el gráfico de memoria.
    let mem_used_pct = if scan.mem_total_kb > 0 {
        (1.0 - scan.mem_avail_kb as f32 / scan.mem_total_kb as f32).clamp(0.0, 1.0) * 100.0
    } else {
        0.0
    };
    push_capped(&mut model.mem_hist, mem_used_pct);

    model.prev_proc = next_prev;
    model.prev_total = scan.total_jiffies;
    model.mem_total_kb = scan.mem_total_kb;
    model.mem_avail_kb = scan.mem_avail_kb;
    model.system = out;
    sort_system(model);

    // El proceso seleccionado pudo morir entre barridos.
    if let Some(sel) = model.sys_sel {
        if !model.system.iter().any(|p| p.pid == sel) {
            model.sys_sel = None;
        }
    }
    // Si la raíz de zoom del mapa murió, salir del zoom.
    if let Some(r) = model.map_root {
        if !model.system.iter().any(|p| p.pid == r) {
            model.map_root = None;
        }
    }
    let max = render_list(model).len().saturating_sub(SYS_ROWS);
    if model.sys_scroll > max {
        model.sys_scroll = max;
    }
}

// ---------------------------------------------------------------------------
// Ordenación, navegación y árbol.
// ---------------------------------------------------------------------------

/// Reajusta el scroll para que la fila seleccionada quede en la ventana visible
/// (según el orden de render actual: lista o árbol).
pub(crate) fn ensure_visible(model: &mut Model) {
    let Some(pid) = model.sys_sel else { return };
    let rows = render_list(model);
    if let Some(i) = rows.iter().position(|r| model.system[r.idx].pid == pid) {
        if i < model.sys_scroll {
            model.sys_scroll = i;
        } else if i >= model.sys_scroll + SYS_ROWS {
            model.sys_scroll = i + 1 - SYS_ROWS;
        }
    }
}

pub(crate) fn sort_system(model: &mut Model) {
    match model.sys_sort {
        Sort::Cpu => model
            .system
            .sort_by(|a, b| b.cpu_pct.total_cmp(&a.cpu_pct)),
        Sort::Mem => model.system.sort_by(|a, b| b.rss_kb.cmp(&a.rss_kb)),
        Sort::Pid => model.system.sort_by(|a, b| a.pid.cmp(&b.pid)),
        Sort::Name => model
            .system
            .sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        // Más viejo primero (mayor uptime arriba).
        Sort::Uptime => model
            .system
            .sort_by(|a, b| b.uptime_secs.cmp(&a.uptime_secs)),
    }
}

/// La lista de filas a pintar/recorrer: plana (modo lista) o aplanada DFS del
/// árbol padre/hijo (modo árbol), respetando los subárboles colapsados. Es la
/// única fuente de orden — render, scroll, navegación ↑↓ comparten esto.
pub(crate) fn render_list(model: &Model) -> Vec<RenderRow> {
    let q = model.sys_filter.trim().to_lowercase();
    // Con filtro activo se aplana a lista plana de coincidencias (filtrar un
    // árbol rompería la jerarquía — comportamiento htop).
    if !q.is_empty() {
        return model
            .system
            .iter()
            .enumerate()
            .filter(|(_, p)| proc_matches(p, &q))
            .map(|(idx, _)| RenderRow {
                idx,
                depth: 0,
                has_kids: false,
            })
            .collect();
    }
    if !model.sys_tree {
        return (0..model.system.len())
            .map(|idx| RenderRow {
                idx,
                depth: 0,
                has_kids: false,
            })
            .collect();
    }
    flatten_tree(&model.system, &model.collapsed)
}

/// Un proceso coincide con `q` (ya en minúsculas) si lo contiene su nombre, su
/// línea de comando o su PID.
pub(crate) fn proc_matches(p: &SysProc, q: &str) -> bool {
    p.name.to_lowercase().contains(q)
        || p.cmd.to_lowercase().contains(q)
        || p.pid.to_string().contains(q)
}

/// Aplana el bosque padre/hijo de `system` (ya ordenado) en orden DFS,
/// saltando los subárboles colapsados. Pura para poder testearla.
pub(crate) fn flatten_tree(system: &[SysProc], collapsed: &HashSet<i32>) -> Vec<RenderRow> {
    // pid → índice (en el orden ya ordenado por sys_sort).
    let pos: HashMap<i32, usize> = system.iter().enumerate().map(|(i, p)| (p.pid, i)).collect();
    // ppid → hijos (índices), preservando el orden global ordenado.
    let mut children: HashMap<i32, Vec<usize>> = HashMap::new();
    let mut roots: Vec<usize> = Vec::new();
    for (i, p) in system.iter().enumerate() {
        if p.ppid != p.pid && p.ppid != 0 && pos.contains_key(&p.ppid) {
            children.entry(p.ppid).or_default().push(i);
        } else {
            roots.push(i);
        }
    }

    let mut out = Vec::with_capacity(system.len());
    let mut seen: HashSet<i32> = HashSet::new();
    // Pila DFS (índice, profundidad); se empuja en reversa para emitir en orden.
    let mut stack: Vec<(usize, u16)> = roots.iter().rev().map(|&i| (i, 0)).collect();
    while let Some((i, depth)) = stack.pop() {
        let pid = system[i].pid;
        if !seen.insert(pid) {
            continue; // guarda anti-ciclo (ppid patológico)
        }
        let kids = children.get(&pid);
        let has_kids = kids.map(|k| !k.is_empty()).unwrap_or(false);
        out.push(RenderRow {
            idx: i,
            depth,
            has_kids,
        });
        if has_kids && !collapsed.contains(&pid) {
            for &c in kids.unwrap().iter().rev() {
                stack.push((c, depth + 1));
            }
        }
    }
    out
}

/// PIDs del subárbol con raíz `root` (incluida), siguiendo `ppid`.
pub(crate) fn subtree_pids(system: &[SysProc], root: i32) -> HashSet<i32> {
    let mut kids: HashMap<i32, Vec<i32>> = HashMap::new();
    for p in system {
        kids.entry(p.ppid).or_default().push(p.pid);
    }
    let mut set = HashSet::new();
    let mut stack = vec![root];
    while let Some(pid) = stack.pop() {
        if set.insert(pid) {
            if let Some(cs) = kids.get(&pid) {
                stack.extend(cs.iter().copied());
            }
        }
    }
    set
}

/// Mueve la selección en la tabla de Sistema siguiendo el **orden de render**
/// (en árbol, recorre la jerarquía aplanada visible).
pub(crate) fn sys_move(model: &Model, dir: i32) -> Option<Msg> {
    let rows = render_list(model);
    if rows.is_empty() {
        return None;
    }
    let cur = model
        .sys_sel
        .and_then(|p| rows.iter().position(|r| model.system[r.idx].pid == p));
    let next = match cur {
        Some(i) => (i as i32 + dir).clamp(0, rows.len() as i32 - 1) as usize,
        None => 0,
    };
    Some(Msg::SysSelect(model.system[rows[next].idx].pid))
}

pub(crate) fn switch_tab(model: &mut Model, tab: Tab, handle: &Handle<Msg>) {
    model.tab = tab;
    match tab {
        Tab::Wawa if model.wawa.is_empty() => {
            handle.spawn(|| Msg::WawaCensus(wawa_census()));
        }
        Tab::System | Tab::Map => handle.spawn(|| Msg::System(super::procfs::scan())),
        _ => {}
    }
}
