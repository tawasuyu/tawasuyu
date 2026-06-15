use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use media_core::config::MediaConfig;
use media_core::color::ColorParams;
use media_core::transform::{Rotation, Transform};
use media_core::control::ControlSettings;
use media_core::layout::{LayoutSettings, PanelId as TileId};
use media_core::library::{Bookmarks, History};

use crate::estado::{
    color, config_file, dynamics, eq, layout_path, playlist_slot, transform, volume,
};
use crate::modelo::Model;
use crate::playlist::{playback_snapshot};
use crate::tipos::{BarEdit, ConfigEdit};

/// Path del `config.ron` unificado.
pub(crate) fn media_config_path() -> Option<PathBuf> {
    config_file("config.ron")
}

/// Config cargada al arrancar, guardada para que `init` la lea sin volver
/// a tocar disco ni el `Playlist`.
pub(crate) fn media_config_slot() -> &'static OnceLock<MediaConfig> {
    static SLOT: OnceLock<MediaConfig> = OnceLock::new();
    &SLOT
}

/// Carga el orden de paneles desde `layout.ron`.
pub(crate) fn load_layout() -> Vec<TileId> {
    let Some(path) = layout_path() else {
        return LayoutSettings::default().panels;
    };
    match std::fs::read_to_string(&path) {
        Ok(body) => match ron::from_str::<LayoutSettings>(&body) {
            Ok(l) => {
                let s = l.sanitized();
                eprintln!("media-app: layout cargado de {}", path.display());
                s.panels
            }
            Err(e) => {
                eprintln!("media-app: layout.ron inválido ({e}) — uso default");
                LayoutSettings::default().panels
            }
        },
        Err(_) => LayoutSettings::default().panels,
    }
}

/// Persiste el orden actual de paneles a `layout.ron`.
pub(crate) fn save_layout(order: &[TileId]) {
    let Some(path) = layout_path() else { return };
    let settings = LayoutSettings {
        panels: order.to_vec(),
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match ron::ser::to_string_pretty(&settings, ron::ser::PrettyConfig::default()) {
        Ok(txt) => {
            if let Err(e) = std::fs::write(&path, txt) {
                eprintln!("media-app: no pude escribir layout: {e}");
            }
        }
        Err(e) => eprintln!("media-app: no pude serializar layout: {e}"),
    }
}

/// Carga los settings de control.
pub(crate) fn load_settings() -> ControlSettings {
    let Some(path) = crate::estado::controles_path() else {
        return ControlSettings::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(body) => match ron::from_str::<ControlSettings>(&body) {
            Ok(s) => {
                eprintln!("media-app: controles cargados de {}", path.display());
                s
            }
            Err(e) => {
                eprintln!(
                    "media-app: controles.ron inválido ({e}) — uso default"
                );
                ControlSettings::default()
            }
        },
        Err(_) => {
            // No existe: sembramos el default para que sea editable.
            let def = ControlSettings::default();
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            match ron::ser::to_string_pretty(&def, ron::ser::PrettyConfig::default()) {
                Ok(txt) => match std::fs::write(&path, txt) {
                    Ok(()) => eprintln!(
                        "media-app: sembré controles default en {}",
                        path.display()
                    ),
                    Err(e) => eprintln!("media-app: no pude escribir controles: {e}"),
                },
                Err(e) => eprintln!("media-app: no pude serializar controles: {e}"),
            }
            def
        }
    }
}

/// Carga el `config.ron` o el default, saneado.
pub(crate) fn load_media_config() -> MediaConfig {
    let Some(path) = media_config_path() else {
        return MediaConfig::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(body) => match ron::from_str::<MediaConfig>(&body) {
            Ok(c) => {
                eprintln!("media-app: config cargada de {}", path.display());
                c.sanitized()
            }
            Err(e) => {
                eprintln!("media-app: config.ron inválido ({e}) — uso default");
                MediaConfig::default()
            }
        },
        Err(_) => {
            let def = MediaConfig::default();
            save_media_config(&def);
            def
        }
    }
}

/// Persiste la config a `config.ron`.
pub(crate) fn save_media_config(cfg: &MediaConfig) {
    let Some(path) = media_config_path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match ron::ser::to_string_pretty(cfg, ron::ser::PrettyConfig::default()) {
        Ok(txt) => {
            if let Err(e) = std::fs::write(&path, txt) {
                eprintln!("media-app: no pude escribir config.ron: {e}");
            }
        }
        Err(e) => eprintln!("media-app: no pude serializar config: {e}"),
    }
}

/// Empuja la config a los handles vivos de la cadena.
pub(crate) fn apply_media_config(cfg: &MediaConfig) {
    // Audio.
    volume().set(cfg.audio.volume);
    eq().set_enabled(cfg.audio.eq_enabled);
    eq().set_all_gains(&cfg.audio.eq_bands_db);
    dynamics().set_enabled(cfg.audio.normalization_enabled);
    // Video (color).
    color().set_enabled(cfg.video.color_enabled);
    color().set_params(ColorParams {
        brightness: cfg.video.brightness,
        contrast: cfg.video.contrast,
        gamma: cfg.video.gamma,
        saturation: cfg.video.saturation,
        hue: cfg.video.hue,
    });
    // Video (orientación) — V3.
    transform().set_transform(Transform {
        rotation: Rotation::from_degrees(cfg.video.rotation),
        flip_h: cfg.video.flip_h,
        flip_v: cfg.video.flip_v,
    });
}

/// Aplica una [`ConfigEdit`] sobre la config.
pub(crate) fn apply_config_edit(cfg: &mut MediaConfig, edit: ConfigEdit) {
    match edit {
        ConfigEdit::VolumeDelta(d) => cfg.audio.volume += d,
        ConfigEdit::ToggleEq => cfg.audio.eq_enabled = !cfg.audio.eq_enabled,
        ConfigEdit::ToggleNormalization => {
            cfg.audio.normalization_enabled = !cfg.audio.normalization_enabled
        }
        ConfigEdit::NormTargetDelta(d) => cfg.audio.normalization_target_lufs += d,
        ConfigEdit::ToggleDownmix => cfg.audio.downmix_to_stereo = !cfg.audio.downmix_to_stereo,
        ConfigEdit::ToggleColor => cfg.video.color_enabled = !cfg.video.color_enabled,
        ConfigEdit::ColorReset => {
            cfg.video.brightness = 0.0;
            cfg.video.contrast = 1.0;
            cfg.video.gamma = 1.0;
            cfg.video.saturation = 1.0;
            cfg.video.hue = 0.0;
        }
        ConfigEdit::BrightnessDelta(d) => cfg.video.brightness += d,
        ConfigEdit::ContrastDelta(d) => cfg.video.contrast += d,
        ConfigEdit::GammaDelta(d) => cfg.video.gamma += d,
        ConfigEdit::SaturationDelta(d) => cfg.video.saturation += d,
        ConfigEdit::HueDelta(d) => cfg.video.hue += d,
        ConfigEdit::RotateCw => cfg.video.rotation = (cfg.video.rotation + 90) % 360,
        ConfigEdit::FlipH => cfg.video.flip_h = !cfg.video.flip_h,
        ConfigEdit::FlipV => cfg.video.flip_v = !cfg.video.flip_v,
        ConfigEdit::ToggleResumeOnOpen => {
            cfg.playlist.resume_on_open = !cfg.playlist.resume_on_open
        }
        ConfigEdit::CycleRepeatDefault => cfg.playlist.repeat = cfg.playlist.repeat.cycle(),
        ConfigEdit::ToggleShuffleDefault => cfg.playlist.shuffle = !cfg.playlist.shuffle,
        ConfigEdit::ToggleAutoloadSidecar => {
            cfg.subtitles.autoload_sidecar = !cfg.subtitles.autoload_sidecar
        }
        ConfigEdit::SubDelayDelta(d) => cfg.subtitles.delay_ms += d,
        ConfigEdit::SubFontDelta(d) => cfg.subtitles.font_scale += d,
        ConfigEdit::CrossfadeDelta(d) => cfg.behavior.crossfade_secs += d,
    }
}

/// Aplica una [`BarEdit`] sobre las barras (y el target).
pub(crate) fn apply_bar_edit(model: &mut Model, edit: BarEdit) {
    let tb = &mut model.config.toolbar;
    match edit {
        BarEdit::AddItem(bar, item) => tb.add_item(bar, item),
        BarEdit::RemoveItem(bar, pos) => {
            tb.remove_item(bar, pos);
        }
        BarEdit::Nudge(bar, pos, dir) => {
            tb.nudge_item(bar, pos, dir);
        }
        BarEdit::AddBar => tb.add_bar(),
        BarEdit::RemoveBar(idx) => {
            tb.remove_bar(idx);
        }
        BarEdit::SetTarget(idx) => model.bar_target = idx,
        BarEdit::TogglePosition(idx) => {
            if let Some(bar) = tb.bars.get_mut(idx) {
                bar.position = bar.position.toggled();
            }
        }
        BarEdit::ToggleEnabled(idx) => {
            tb.toggle_enabled(idx);
        }
        BarEdit::ToggleAutohide(idx) => {
            tb.toggle_autohide(idx);
        }
    }
}

pub(crate) fn history_path() -> Option<PathBuf> {
    config_file("history.ron")
}

pub(crate) fn load_history() -> History {
    let Some(p) = history_path() else {
        return History::default();
    };
    match std::fs::read_to_string(&p) {
        Ok(body) => ron::from_str::<History>(&body)
            .map(History::sanitized)
            .unwrap_or_default(),
        Err(_) => History::default(),
    }
}

/// Persiste el historial a `history.ron` (best-effort, sólo log).
pub(crate) fn save_history() {
    let Some(p) = history_path() else {
        return;
    };
    let snapshot = history().lock().clone();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(txt) = ron::ser::to_string_pretty(&snapshot, ron::ser::PrettyConfig::default()) {
        let _ = std::fs::write(&p, txt);
    }
}

pub(crate) fn bookmarks_path() -> Option<PathBuf> {
    config_file("bookmarks.ron")
}

pub(crate) fn load_bookmarks() -> Bookmarks {
    let Some(p) = bookmarks_path() else {
        return Bookmarks::default();
    };
    match std::fs::read_to_string(&p) {
        Ok(body) => ron::from_str::<Bookmarks>(&body)
            .map(Bookmarks::sanitized)
            .unwrap_or_default(),
        Err(_) => Bookmarks::default(),
    }
}

/// Persiste las marcas a `bookmarks.ron` (best-effort, sólo log).
pub(crate) fn save_bookmarks() {
    let Some(p) = bookmarks_path() else {
        return;
    };
    let snapshot = bookmarks().lock().clone();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(txt) = ron::ser::to_string_pretty(&snapshot, ron::ser::PrettyConfig::default()) {
        let _ = std::fs::write(&p, txt);
    }
}

/// Época Unix en segundos (para la recencia del historial).
pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Historial de reproducción global (resume por medio).
pub(crate) fn history() -> &'static parking_lot::Mutex<History> {
    static SLOT: OnceLock<parking_lot::Mutex<History>> = OnceLock::new();
    SLOT.get_or_init(|| parking_lot::Mutex::new(load_history()))
}

/// Marcas manuales de toda la biblioteca (U6).
pub(crate) fn bookmarks() -> &'static parking_lot::Mutex<Bookmarks> {
    static SLOT: OnceLock<parking_lot::Mutex<Bookmarks>> = OnceLock::new();
    SLOT.get_or_init(|| parking_lot::Mutex::new(load_bookmarks()))
}

/// Aplica TODO el arranque que depende del `Playlist`.
pub(crate) fn apply_startup_config() {
    let config = load_media_config();
    apply_media_config(&config);
    if let Some(h) = playlist_slot().get().and_then(|o| o.as_ref()) {
        let mut pl = h.lock();
        pl.set_repeat(repeat_mode_from(config.playlist.repeat));
        if config.playlist.shuffle && !pl.shuffle_on() {
            pl.toggle_shuffle();
        }
    }
    if config.playlist.resume_on_open {
        let key = playlist_slot()
            .get()
            .and_then(|o| o.as_ref())
            .map(|h| h.lock().track_path().to_string_lossy().into_owned());
        if let Some(key) = key {
            let resume = history()
                .lock()
                .resume_position(&key, std::time::Duration::from_secs(5));
            if let Some(pos) = resume {
                crate::playlist::seek_audio_to_pos(pos);
            }
            history().lock().note_play(&key, now_secs());
        }
    }
    if let Some(p) = crate::estado::current_media_path().filter(|p| p.is_file()) {
        let _ = crate::estado::media_metadata_slot().set(crate::media_io::load_media_metadata(&p));
        let _ = crate::estado::chapters_slot().set(crate::media_io::load_chapters(&p));
    }
    let _ = media_config_slot().set(config);
}

/// Mapea el modo de repetición de la config (`media-core`) al de la cola viva.
pub(crate) fn repeat_mode_from(r: media_core::playlist::Repeat) -> crate::playlist::RepeatMode {
    use media_core::playlist::Repeat;
    use crate::playlist::RepeatMode;
    match r {
        Repeat::Off => RepeatMode::Off,
        Repeat::One => RepeatMode::One,
        Repeat::All => RepeatMode::All,
    }
}
