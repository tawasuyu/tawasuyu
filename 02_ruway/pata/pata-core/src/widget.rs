//! El modelo de widget de `pata`: **lógica de datos sin pincel**.
//!
//! Un [`WidgetSpec`] (lo que el config declara) se materializa en un objeto
//! [`Widget`] vivo. El widget no sabe dibujar: cada `tick` refresca su estado a
//! partir de un [`WidgetCtx`] —un snapshot agnóstico del sistema que el host
//! muestrea (reloj, CPU, RAM, volumen, brillo…)— y `view` emite un
//! [`WidgetView`], un view-model que describe *qué* mostrar (texto, medidor,
//! placeholder) sin decir *cómo*. El frontend (Llimphi en Linux, framebuffer en
//! wawa) traduce ese view-model a su pincel.
//!
//! La frontera está donde tiene que estar: el core es `no_std` y determinista,
//! así que **no lee el reloj ni los contadores del kernel** —eso son syscalls—.
//! El host los muestrea y los entrega en el [`WidgetCtx`]; el core sólo formatea
//! y compone. Misma lógica de datos para los dos mundos; cada uno aporta su
//! sampler y su pincel.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::config::WidgetSpec;

/// Lectura del reloj descompuesta. El host la rellena desde su fuente de tiempo
/// (en Linux, la zona horaria de `general.timezone`); el core sólo la formatea.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClockReading {
    /// Año con siglo (p. ej. `2026`).
    pub year: u16,
    /// Mes `1..=12`.
    pub month: u8,
    /// Día del mes `1..=31`.
    pub day: u8,
    /// Día de la semana, `0` = domingo … `6` = sábado.
    pub weekday: u8,
    /// Hora `0..=23`.
    pub hour: u8,
    /// Minuto `0..=59`.
    pub minute: u8,
    /// Segundo `0..=59`.
    pub second: u8,
}

/// El snapshot del sistema que alimenta a los widgets en cada `tick`. El host
/// lo muestrea (vía sysfs/PulseAudio en Linux, vía el kernel en wawa) y lo pasa
/// por valor: el core no toca el SO. Todos los campos arrancan en cero, así que
/// un frontend puede llenar sólo lo que le importe.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WidgetCtx {
    /// Hora actual ya descompuesta.
    pub clock: ClockReading,
    /// Uso de CPU, fracción `0.0..=1.0`.
    pub cpu: f32,
    /// Uso de RAM, fracción `0.0..=1.0`.
    pub ram: f32,
    /// RAM usada en MiB (para la leyenda del medidor).
    pub ram_used_mb: u32,
    /// RAM total en MiB.
    pub ram_total_mb: u32,
    /// Volumen, fracción `0.0..=1.0`.
    pub volume: f32,
    /// `true` si el audio está silenciado.
    pub muted: bool,
    /// Brillo de pantalla, fracción `0.0..=1.0`.
    pub brightness: f32,
}

/// El view-model que un widget emite: describe qué pintar sin atarse a ningún
/// pincel. El frontend hace el match y lo traduce a su backend gráfico.
#[derive(Debug, Clone, PartialEq)]
pub enum WidgetView {
    /// Nada que pintar (un widget que aún no tiene datos).
    Empty,
    /// Una línea de texto: el reloj, una etiqueta.
    Text(String),
    /// Un medidor: `fraction` en `0.0..=1.0`, una `caption` ya formateada y una
    /// `label` opcional (el nombre corto, p. ej. `"CPU"`).
    Meter {
        /// Etiqueta corta, o `None` si el widget la oculta.
        label: Option<String>,
        /// Fracción `0.0..=1.0` que el frontend pinta como barra/arco.
        fraction: f32,
        /// Leyenda ya formateada (`"42%"`, `"3.2G"`, `"muted"`).
        caption: String,
    },
    /// Un widget cuyo `kind` el core no implementa todavía: el frontend pinta un
    /// chip tenue con este nombre. Permite encodear la visión completa del marco
    /// (start_button, tray, astro…) antes de que cada widget exista.
    Placeholder(String),
}

/// Un widget vivo: refresca su estado en cada `tick` y emite su view-model en
/// `view`. La lógica de datos vive acá; el dibujo, en el frontend.
pub trait Widget {
    /// Refresca el estado interno con el snapshot del sistema.
    fn tick(&mut self, ctx: &WidgetCtx);
    /// El view-model actual.
    fn view(&self) -> WidgetView;
}

/// Reloj: formatea [`ClockReading`] según una cadena estilo `strftime` reducida.
///
/// Tokens soportados (suficientes para una barra): `%H %M %S` (hora/min/seg a
/// dos dígitos), `%I %p` (12h + AM/PM), `%d %m %Y %y` (día/mes/año), `%%`
/// (porcentaje literal). Cualquier otro carácter pasa tal cual. No es un
/// `strftime` completo a propósito: nombres de mes/día localizados los resuelve
/// `rimay-localize` aguas arriba, no este core.
#[derive(Debug, Clone)]
pub struct Clock {
    format: String,
    text: String,
}

impl Clock {
    /// Construye desde el spec leyendo la prop `format` (default `"%H:%M"`).
    pub fn from_spec(spec: &WidgetSpec) -> Self {
        Self {
            format: spec.str_prop("format", "%H:%M").to_string(),
            text: String::new(),
        }
    }
}

impl Widget for Clock {
    fn tick(&mut self, ctx: &WidgetCtx) {
        self.text = format_time(&self.format, &ctx.clock);
    }

    fn view(&self) -> WidgetView {
        if self.text.is_empty() {
            WidgetView::Empty
        } else {
            WidgetView::Text(self.text.clone())
        }
    }
}

/// De dónde saca su valor un [`Meter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeterSource {
    /// Uso de CPU.
    Cpu,
    /// Uso de RAM (leyenda en GiB usados/total).
    Ram,
    /// Volumen de audio (leyenda `"muted"` si está silenciado).
    Volume,
    /// Brillo de pantalla.
    Brightness,
}

impl MeterSource {
    /// La etiqueta corta por defecto de la fuente.
    fn label_por_defecto(&self) -> &'static str {
        match self {
            MeterSource::Cpu => "CPU",
            MeterSource::Ram => "RAM",
            MeterSource::Volume => "VOL",
            MeterSource::Brightness => "BRI",
        }
    }
}

/// Medidor genérico: lee una fracción `0..1` del [`WidgetCtx`] según su
/// [`MeterSource`] y arma una leyenda. Cubre cpu/ram/volumen/brillo con la
/// misma lógica; el frontend decide si lo pinta como barra, arco o ícono.
#[derive(Debug, Clone)]
pub struct Meter {
    source: MeterSource,
    label: Option<String>,
    fraction: f32,
    caption: String,
}

impl Meter {
    /// Construye un medidor de `source` leyendo del spec:
    /// - `label` (string): override de la etiqueta corta;
    /// - `show_label` (bool, default `true`): si es `false`, oculta la etiqueta.
    pub fn from_spec(source: MeterSource, spec: &WidgetSpec) -> Self {
        let label = if spec.bool_prop("show_label", true) {
            Some(
                spec.str_prop("label", source.label_por_defecto())
                    .to_string(),
            )
        } else {
            None
        };
        Self {
            source,
            label,
            fraction: 0.0,
            caption: String::new(),
        }
    }
}

impl Widget for Meter {
    fn tick(&mut self, ctx: &WidgetCtx) {
        match self.source {
            MeterSource::Cpu => {
                self.fraction = ctx.cpu;
                self.caption = porcentaje(ctx.cpu);
            }
            MeterSource::Ram => {
                self.fraction = ctx.ram;
                self.caption = leyenda_memoria(ctx.ram_used_mb, ctx.ram_total_mb);
            }
            MeterSource::Volume => {
                self.fraction = ctx.volume;
                self.caption = if ctx.muted {
                    "muted".to_string()
                } else {
                    porcentaje(ctx.volume)
                };
            }
            MeterSource::Brightness => {
                self.fraction = ctx.brightness;
                self.caption = porcentaje(ctx.brightness);
            }
        }
    }

    fn view(&self) -> WidgetView {
        WidgetView::Meter {
            label: self.label.clone(),
            fraction: self.fraction.clamp(0.0, 1.0),
            caption: self.caption.clone(),
        }
    }
}

/// Widget de relleno para un `kind` que el core no implementa todavía. Su `view`
/// es siempre un [`WidgetView::Placeholder`] con el nombre del kind.
#[derive(Debug, Clone)]
pub struct Placeholder {
    kind: String,
}

impl Placeholder {
    /// Un placeholder que muestra `kind`.
    pub fn new(kind: impl Into<String>) -> Self {
        Self { kind: kind.into() }
    }
}

impl Widget for Placeholder {
    fn tick(&mut self, _ctx: &WidgetCtx) {}

    fn view(&self) -> WidgetView {
        WidgetView::Placeholder(self.kind.clone())
    }
}

/// Materializa un [`WidgetSpec`] en un [`Widget`] vivo. Los `kind`s que el core
/// ya implementa (reloj y medidores) se construyen con su lógica; el resto cae a
/// un [`Placeholder`] —el conjunto de kinds es abierto, así que esto nunca
/// falla—. Los widgets que dependen de IPC o crates externos (`window_list`,
/// `astro`, `tray`, `shuma_input`) llegan en fases posteriores.
pub fn build(spec: &WidgetSpec) -> Box<dyn Widget> {
    match spec.kind.as_str() {
        "clock" => Box::new(Clock::from_spec(spec)),
        "cpu_meter" => Box::new(Meter::from_spec(MeterSource::Cpu, spec)),
        "ram_meter" => Box::new(Meter::from_spec(MeterSource::Ram, spec)),
        "volume" => Box::new(Meter::from_spec(MeterSource::Volume, spec)),
        "brightness" => Box::new(Meter::from_spec(MeterSource::Brightness, spec)),
        _ => Box::new(Placeholder::new(&spec.kind)),
    }
}

/// Materializa una lista de specs (un slot completo) de una pasada.
pub fn build_all(specs: &[WidgetSpec]) -> Vec<Box<dyn Widget>> {
    specs.iter().map(build).collect()
}

/// Formatea un [`ClockReading`] con la cadena `fmt` (subconjunto de `strftime`,
/// ver [`Clock`]).
fn format_time(fmt: &str, t: &ClockReading) -> String {
    let mut out = String::with_capacity(fmt.len() + 4);
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('H') => empuja_dos(&mut out, t.hour),
            Some('M') => empuja_dos(&mut out, t.minute),
            Some('S') => empuja_dos(&mut out, t.second),
            Some('I') => {
                let h12 = match t.hour % 12 {
                    0 => 12,
                    h => h,
                };
                empuja_dos(&mut out, h12);
            }
            Some('p') => out.push_str(if t.hour < 12 { "AM" } else { "PM" }),
            Some('d') => empuja_dos(&mut out, t.day),
            Some('m') => empuja_dos(&mut out, t.month),
            Some('Y') => out.push_str(&format!("{:04}", t.year)),
            Some('y') => empuja_dos(&mut out, (t.year % 100) as u8),
            Some('%') => out.push('%'),
            // Token desconocido: lo dejamos literal (`%` + el carácter).
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

/// Empuja `n` como dos dígitos con cero a la izquierda.
fn empuja_dos(out: &mut String, n: u8) {
    out.push_str(&format!("{:02}", n));
}

/// Una fracción `0..1` como porcentaje entero: `0.42 → "42%"`.
fn porcentaje(frac: f32) -> String {
    // `f32::round` vive en `std`; acá (no_std) redondeamos a mano. El valor es
    // siempre ≥ 0 (fracción clampeada), así que `+ 0.5` y truncar basta.
    let pct = (frac.clamp(0.0, 1.0) * 100.0 + 0.5) as i32;
    format!("{}%", pct)
}

/// Leyenda de memoria `"usado/total"` en GiB con un decimal: `"3.2/15.5G"`.
fn leyenda_memoria(used_mb: u32, total_mb: u32) -> String {
    let used = used_mb as f32 / 1024.0;
    let total = total_mb as f32 / 1024.0;
    format!("{:.1}/{:.1}G", used, total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Prop;

    fn ctx() -> WidgetCtx {
        WidgetCtx {
            clock: ClockReading {
                year: 2026,
                month: 6,
                day: 1,
                weekday: 1,
                hour: 14,
                minute: 7,
                second: 9,
            },
            cpu: 0.42,
            ram: 0.5,
            ram_used_mb: 3277, // ~3.2 GiB
            ram_total_mb: 15872,
            volume: 0.75,
            muted: false,
            brightness: 0.3,
            ..WidgetCtx::default()
        }
    }

    #[test]
    fn reloj_formatea_default_y_segundos() {
        let mut c = Clock::from_spec(&WidgetSpec::new("clock"));
        c.tick(&ctx());
        assert_eq!(c.view(), WidgetView::Text("14:07".to_string()));

        let mut con_seg = Clock::from_spec(
            &WidgetSpec::new("clock").with("format", Prop::Str("%H:%M:%S".to_string())),
        );
        con_seg.tick(&ctx());
        assert_eq!(con_seg.view(), WidgetView::Text("14:07:09".to_string()));
    }

    #[test]
    fn reloj_12h_fecha_y_literales() {
        let spec = WidgetSpec::new("clock")
            .with("format", Prop::Str("%d/%m/%Y %I:%M %p".to_string()));
        let mut c = Clock::from_spec(&spec);
        c.tick(&ctx());
        assert_eq!(c.view(), WidgetView::Text("01/06/2026 02:07 PM".to_string()));

        // %% literal y token desconocido pasa tal cual.
        let mut raro = Clock::from_spec(
            &WidgetSpec::new("clock").with("format", Prop::Str("%H%% %q".to_string())),
        );
        raro.tick(&ctx());
        assert_eq!(raro.view(), WidgetView::Text("14% %q".to_string()));
    }

    #[test]
    fn medianoche_en_12h_es_las_12() {
        let mut t = ctx();
        t.clock.hour = 0;
        let mut c = Clock::from_spec(
            &WidgetSpec::new("clock").with("format", Prop::Str("%I %p".to_string())),
        );
        c.tick(&t);
        assert_eq!(c.view(), WidgetView::Text("12 AM".to_string()));
    }

    #[test]
    fn cpu_meter_emite_fraccion_y_porcentaje() {
        let mut m = Meter::from_spec(MeterSource::Cpu, &WidgetSpec::new("cpu_meter"));
        m.tick(&ctx());
        assert_eq!(
            m.view(),
            WidgetView::Meter {
                label: Some("CPU".to_string()),
                fraction: 0.42,
                caption: "42%".to_string(),
            }
        );
    }

    #[test]
    fn ram_meter_leyenda_en_gib() {
        let mut m = Meter::from_spec(MeterSource::Ram, &WidgetSpec::new("ram_meter"));
        m.tick(&ctx());
        match m.view() {
            WidgetView::Meter { caption, .. } => assert_eq!(caption, "3.2/15.5G"),
            v => panic!("esperaba Meter, vino {v:?}"),
        }
    }

    #[test]
    fn volumen_muteado_dice_muted() {
        let mut t = ctx();
        t.muted = true;
        let mut m = Meter::from_spec(MeterSource::Volume, &WidgetSpec::new("volume"));
        m.tick(&t);
        match m.view() {
            WidgetView::Meter { caption, .. } => assert_eq!(caption, "muted"),
            v => panic!("esperaba Meter, vino {v:?}"),
        }
    }

    #[test]
    fn meter_oculta_label_con_show_label_false() {
        let spec = WidgetSpec::new("cpu_meter").with("show_label", Prop::Bool(false));
        let mut m = Meter::from_spec(MeterSource::Cpu, &spec);
        m.tick(&ctx());
        match m.view() {
            WidgetView::Meter { label, .. } => assert_eq!(label, None),
            v => panic!("esperaba Meter, vino {v:?}"),
        }
    }

    #[test]
    fn meter_label_override() {
        let spec = WidgetSpec::new("cpu_meter").with("label", Prop::Str("Proc".to_string()));
        let m = Meter::from_spec(MeterSource::Cpu, &spec);
        match m.view() {
            WidgetView::Meter { label, .. } => assert_eq!(label, Some("Proc".to_string())),
            v => panic!("esperaba Meter, vino {v:?}"),
        }
    }

    #[test]
    fn kind_desconocido_cae_a_placeholder() {
        let w = build(&WidgetSpec::new("start_button"));
        assert_eq!(w.view(), WidgetView::Placeholder("start_button".to_string()));
        // astro/window_list/tray/shuma_input todavía no son builtin.
        assert_eq!(
            build(&WidgetSpec::new("astro")).view(),
            WidgetView::Placeholder("astro".to_string())
        );
    }

    #[test]
    fn build_despacha_los_builtins() {
        let specs = [
            WidgetSpec::new("clock"),
            WidgetSpec::new("cpu_meter"),
            WidgetSpec::new("ram_meter"),
            WidgetSpec::new("volume"),
            WidgetSpec::new("brightness"),
        ];
        let mut widgets = build_all(&specs);
        for w in widgets.iter_mut() {
            w.tick(&ctx());
        }
        // El reloj da texto; los cuatro medidores dan Meter.
        assert!(matches!(widgets[0].view(), WidgetView::Text(_)));
        for w in &widgets[1..] {
            assert!(matches!(w.view(), WidgetView::Meter { .. }));
        }
    }
}
