//! Lectura cruda de `/proc` para el modo **Sistema** del monitor.
//!
//! Esto NO es la observación del plano de control (ése va por el contrato
//! `Engine`, ver `shared/sandokan/SDD.md` §6). El modo Sistema observa el SO
//! entero —todos los procesos del kernel, no sólo las unidades que sandokan
//! gestiona— que es una fuente distinta y sin dueño en el control plane: no
//! hay "segunda fuente de verdad" porque el Engine no pretende cubrir procesos
//! que no encarnó. Es, literalmente, un `htop` mínimo: leer `/proc`, calcular
//! %CPU por deltas de jiffies, y mandar señales con `nix`.

use std::fs;
use std::os::unix::fs::MetadataExt;

/// Lectura puntual de un proceso (jiffies crudos; el %CPU se deriva luego por
/// delta contra la lectura previa).
#[derive(Clone)]
pub struct ProcRaw {
    pub pid: i32,
    pub ppid: i32,
    pub name: String,
    pub state: char,
    pub rss_kb: u64,
    pub threads: u32,
    /// `utime + stime` acumulados (jiffies). El %CPU es el delta sobre el
    /// delta del total de la CPU.
    pub cpu_jiffies: u64,
    pub uid: u32,
    pub cmd: String,
}

/// Un barrido completo de `/proc` + denominadores para normalizar.
#[derive(Clone)]
pub struct Scan {
    pub procs: Vec<ProcRaw>,
    /// Jiffies totales de CPU (suma de la línea `cpu` de `/proc/stat`).
    pub total_jiffies: u64,
    pub ncpu: u32,
    pub mem_total_kb: u64,
    /// Memoria disponible (`MemAvailable`), para derivar la usada.
    pub mem_avail_kb: u64,
    /// Por core: `(jiffies_totales, jiffies_idle)` para derivar %uso por delta.
    pub cores: Vec<(u64, u64)>,
}

impl Default for Scan {
    fn default() -> Self {
        Scan {
            procs: Vec::new(),
            total_jiffies: 0,
            ncpu: 1,
            mem_total_kb: 0,
            mem_avail_kb: 0,
            cores: Vec::new(),
        }
    }
}

fn page_kb() -> u64 {
    // SAFETY: sysconf sin estado, sin punteros; devuelve bytes por página.
    let bytes = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if bytes > 0 {
        (bytes as u64) / 1024
    } else {
        4
    }
}

/// `(total, idle)` de una línea `cpu...` de `/proc/stat`. `idle = idle +
/// iowait` (campos 4 y 5), como htop.
fn parse_cpu_line(line: &str) -> (u64, u64) {
    let nums: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|n| n.parse::<u64>().ok())
        .collect();
    let total: u64 = nums.iter().sum();
    let idle = nums.get(3).copied().unwrap_or(0) + nums.get(4).copied().unwrap_or(0);
    (total, idle)
}

/// Línea agregada `cpu ` (total de jiffies) + una entrada `(total, idle)` por
/// core (`cpu0`, `cpu1`, …).
fn cpu_stat() -> (u64, Vec<(u64, u64)>) {
    let Ok(stat) = fs::read_to_string("/proc/stat") else {
        return (0, Vec::new());
    };
    let mut total = 0;
    let mut cores = Vec::new();
    for line in stat.lines() {
        if let Some(rest) = line.strip_prefix("cpu") {
            if rest.starts_with(char::is_whitespace) {
                total = parse_cpu_line(line).0; // línea agregada `cpu `
            } else if rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                cores.push(parse_cpu_line(line)); // `cpuN`
            }
        } else {
            break; // las líneas cpu van primero
        }
    }
    (total, cores)
}

/// `(MemTotal, MemAvailable)` en kB desde `/proc/meminfo`.
fn meminfo_kb() -> (u64, u64) {
    let Ok(mi) = fs::read_to_string("/proc/meminfo") else {
        return (0, 0);
    };
    let field = |prefix: &str| -> u64 {
        mi.lines()
            .find_map(|l| l.strip_prefix(prefix))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|n| n.parse::<u64>().ok())
            .unwrap_or(0)
    };
    (field("MemTotal:"), field("MemAvailable:"))
}

/// Parsea `/proc/<pid>/stat`. El `comm` puede traer espacios y paréntesis, así
/// que se aísla entre el primer `(` y el último `)`; el resto se tokeniza.
fn parse_one(pid: i32, page: u64) -> Option<ProcRaw> {
    let base = format!("/proc/{pid}");
    let stat = fs::read_to_string(format!("{base}/stat")).ok()?;
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let name = stat.get(open + 1..close)?.to_string();
    // Tras `) ` empieza el campo 3 (estado); `f[k]` = campo `k + 3`.
    let rest = stat.get(close + 2..)?;
    let f: Vec<&str> = rest.split_whitespace().collect();
    let state = f.first().and_then(|s| s.chars().next()).unwrap_or('?');
    let ppid: i32 = f.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let utime: u64 = f.get(11).and_then(|s| s.parse().ok()).unwrap_or(0);
    let stime: u64 = f.get(12).and_then(|s| s.parse().ok()).unwrap_or(0);
    let threads: u32 = f.get(17).and_then(|s| s.parse().ok()).unwrap_or(1);

    // RSS desde statm (campo 2 = residente, en páginas).
    let rss_kb = fs::read_to_string(format!("{base}/statm"))
        .ok()
        .and_then(|s| {
            s.split_whitespace()
                .nth(1)
                .and_then(|n| n.parse::<u64>().ok())
        })
        .map(|pages| pages * page)
        .unwrap_or(0);

    let uid = fs::metadata(&base).map(|m| m.uid()).unwrap_or(0);

    // Línea de comando completa (argv NUL-separado); cae al `comm` si está
    // vacía (kernel threads).
    let cmd = fs::read(format!("{base}/cmdline"))
        .ok()
        .map(|bytes| {
            let s: String = bytes
                .split(|b| *b == 0)
                .filter(|p| !p.is_empty())
                .map(|p| String::from_utf8_lossy(p))
                .collect::<Vec<_>>()
                .join(" ");
            s
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("[{name}]"));

    Some(ProcRaw {
        pid,
        ppid,
        name,
        state,
        rss_kb,
        threads,
        cpu_jiffies: utime + stime,
        uid,
        cmd,
    })
}

/// Barre `/proc` entero. Pensado para correr en un hilo de fondo (es IO puro);
/// el delta de %CPU lo hace el `update` con la lectura previa.
pub fn scan() -> Scan {
    let page = page_kb();
    let mut procs = Vec::new();
    if let Ok(rd) = fs::read_dir("/proc") {
        for ent in rd.flatten() {
            let name = ent.file_name();
            let Some(s) = name.to_str() else { continue };
            let Ok(pid) = s.parse::<i32>() else { continue };
            if let Some(p) = parse_one(pid, page) {
                procs.push(p);
            }
        }
    }
    let (mem_total_kb, mem_avail_kb) = meminfo_kb();
    let (total_jiffies, cores) = cpu_stat();
    let ncpu = if cores.is_empty() {
        std::thread::available_parallelism()
            .map(|n| n.get() as u32)
            .unwrap_or(1)
    } else {
        cores.len() as u32
    };
    Scan {
        procs,
        total_jiffies,
        ncpu,
        mem_total_kb,
        mem_avail_kb,
        cores,
    }
}

/// Señal a enviar a un PID desde la barra de acciones.
#[derive(Clone, Copy)]
pub enum Sig {
    Term,
    Kill,
    Stop,
    Cont,
}

/// Envía la señal. Devuelve `Err(texto)` si falla (p. ej. permiso denegado).
pub fn signal(pid: i32, sig: Sig) -> Result<(), String> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    let s = match sig {
        Sig::Term => Signal::SIGTERM,
        Sig::Kill => Signal::SIGKILL,
        Sig::Stop => Signal::SIGSTOP,
        Sig::Cont => Signal::SIGCONT,
    };
    kill(Pid::from_raw(pid), s).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_line_total_e_idle() {
        // user nice system idle iowait irq softirq steal ...
        let (total, idle) = parse_cpu_line("cpu0 100 0 50 800 20 0 0 0 0 0");
        assert_eq!(total, 100 + 0 + 50 + 800 + 20);
        assert_eq!(idle, 800 + 20); // idle + iowait
    }

    #[test]
    fn scan_lee_cores() {
        let s = scan();
        assert!(!s.cores.is_empty(), "debería leer al menos un core");
        assert_eq!(s.ncpu as usize, s.cores.len());
        assert!(s.cores.iter().all(|&(t, i)| t >= i));
    }

    #[test]
    fn scan_ve_procesos_y_se_encuentra_a_si_mismo() {
        let s = scan();
        assert!(!s.procs.is_empty(), "debería ver procesos");
        assert!(s.total_jiffies > 0, "jiffies de /proc/stat");
        assert!(s.mem_total_kb > 0, "MemTotal de /proc/meminfo");
        let me = std::process::id() as i32;
        let yo = s.procs.iter().find(|p| p.pid == me).expect("mi propio pid");
        assert!(yo.threads >= 1);
        assert!(!yo.cmd.is_empty());
    }
}
