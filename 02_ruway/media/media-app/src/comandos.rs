use std::sync::Arc;
use std::time::Duration;

use llimphi_ui::{Handle, Key, KeyEvent, KeyState, NamedKey};
use media_core::control::{ColorParam, ControlSettings, KeyChord, MediaCommand};
use media_core::eq::ISO_10_BANDS_HZ;
use media_core::loudness::REPLAYGAIN_TARGET_LUFS;
use media_core::osd;
use media_recorder_wav::default_recording_path;
use llimphi_module_command_palette::{
    self as palette, Command as PaletteCommand, PaletteAction, PaletteMsg, PaletteState,
};
use parking_lot::Mutex;

use crate::estado::{
    color, chapters_slot, dynamics, eq, ffmpeg_session_slot, loudness, muted_volume,
    osd_flash, osd_flash_seek, osd_now, osd, pause, pipeline_slot, playlist_slot,
    recorder, subtitles_slot, tracks, transform, video_path_slot, viewcontrol,
    volume, SUB_DELAY_MS, MAX_SUB_DELAY_MS,
};
use crate::playlist::{
    current_track_key, jump_playlist_to, playback_snapshot, seek_audio_by,
    seek_audio_to, seek_audio_to_pos, RepeatMode,
};
use crate::media_io::media_title_string;
use crate::tipos::Msg;

use std::sync::atomic::Ordering;

/// Construye el catálogo de acciones para el command palette.
pub(crate) fn build_command_catalog(s: &ControlSettings) -> (Vec<PaletteCommand>, Vec<MediaCommand>) {
    use MediaCommand::*;
    let step = s.seek_step_secs;
    let vstep = s.volume_step;
    let acciones: Vec<(MediaCommand, &str)> = vec![
        (TogglePause, "Transporte"),
        (SeekBy { secs: step }, "Transporte"),
        (SeekBy { secs: -step }, "Transporte"),
        (SeekTo { fraction: 0.0 }, "Transporte"),
        (PrevTrack, "Playlist"),
        (NextTrack, "Playlist"),
        (ChapterPrev, "Capítulos"),
        (ChapterNext, "Capítulos"),
        (CycleRepeat, "Playlist"),
        (ToggleShuffle, "Playlist"),
        (VolumeBy { delta: vstep }, "Volumen"),
        (VolumeBy { delta: -vstep }, "Volumen"),
        (SetVolume { level: 1.0 }, "Volumen"),
        (SetVolume { level: 0.5 }, "Volumen"),
        (SetVolume { level: 0.0 }, "Volumen"),
        (ToggleMute, "Volumen"),
        (SpeedStep { dir: 1 }, "Velocidad"),
        (SpeedStep { dir: -1 }, "Velocidad"),
        (SetSpeed { mult: 1.0 }, "Velocidad"),
        (FrameStep { dir: 1 }, "Transporte"),
        (FrameStep { dir: -1 }, "Transporte"),
        (EqToggle, "Ecualizador"),
        (EqReset, "Ecualizador"),
        (AvSyncBy { ms: -50 }, "Sync A/V"),
        (AvSyncBy { ms: 50 }, "Sync A/V"),
        (AvSyncReset, "Sync A/V"),
        (ColorToggle, "Color"),
        (ColorReset, "Color"),
        (ColorBy { param: ColorParam::Brightness, delta: 0.05 }, "Color"),
        (ColorBy { param: ColorParam::Brightness, delta: -0.05 }, "Color"),
        (ColorBy { param: ColorParam::Contrast, delta: 0.1 }, "Color"),
        (ColorBy { param: ColorParam::Contrast, delta: -0.1 }, "Color"),
        (ColorBy { param: ColorParam::Gamma, delta: 0.1 }, "Color"),
        (ColorBy { param: ColorParam::Gamma, delta: -0.1 }, "Color"),
        (ColorBy { param: ColorParam::Saturation, delta: 0.1 }, "Color"),
        (ColorBy { param: ColorParam::Saturation, delta: -0.1 }, "Color"),
        (ColorBy { param: ColorParam::Hue, delta: 10.0 }, "Color"),
        (ColorBy { param: ColorParam::Hue, delta: -10.0 }, "Color"),
        (RotateBy { dir: 1 }, "Orientación"),
        (RotateBy { dir: -1 }, "Orientación"),
        (FlipH, "Orientación"),
        (FlipV, "Orientación"),
        (OrientReset, "Orientación"),
        (ViewCycleFit, "Imagen"),
        (ViewZoomBy { factor: 1.1 }, "Imagen"),
        (ViewZoomBy { factor: 0.9 }, "Imagen"),
        (ViewPanBy { dx: -0.05, dy: 0.0 }, "Imagen"),
        (ViewPanBy { dx: 0.05, dy: 0.0 }, "Imagen"),
        (ViewPanBy { dx: 0.0, dy: -0.05 }, "Imagen"),
        (ViewPanBy { dx: 0.0, dy: 0.05 }, "Imagen"),
        (ViewReset, "Imagen"),
        (SubDelayBy { ms: -100 }, "Subtítulos"),
        (SubDelayBy { ms: 100 }, "Subtítulos"),
        (SubDelayReset, "Subtítulos"),
        (NormToggle, "Normalización"),
        (NormAuto, "Normalización"),
        (NormGainBy { db: 3.0 }, "Normalización"),
        (NormGainBy { db: -3.0 }, "Normalización"),
        (NormReset, "Normalización"),
        (Snapshot, "Captura"),
        (ToggleRecord, "Captura"),
        (BookmarkToggle, "Marcas"),
        (BookmarkNext, "Marcas"),
        (BookmarkPrev, "Marcas"),
        (CycleAudioTrack, "Pistas"),
        (CycleSubtitleTrack, "Pistas"),
    ];
    let mut catalog = Vec::with_capacity(acciones.len());
    let mut cmds = Vec::with_capacity(acciones.len());
    for (i, (cmd, group)) in acciones.into_iter().enumerate() {
        let mut pc = PaletteCommand::new(i.to_string(), cmd.describe(), group);
        if let Some(sc) = shortcut_for(&s.keymap, &cmd) {
            pc = pc.with_shortcut(sc);
        }
        catalog.push(pc);
        cmds.push(cmd);
    }
    for ns in &s.scripts {
        let cmd = MediaCommand::Script {
            name: ns.name.clone(),
        };
        let id = cmds.len();
        let mut pc = PaletteCommand::new(id.to_string(), cmd.describe(), "Scripts");
        if let Some(sc) = shortcut_for(&s.keymap, &cmd) {
            pc = pc.with_shortcut(sc);
        }
        catalog.push(pc);
        cmds.push(cmd);
    }
    for idx in 0..ISO_10_BANDS_HZ.len() {
        for delta_db in [3.0_f32, -3.0] {
            let cmd = MediaCommand::EqBandBy { idx, delta_db };
            let id = cmds.len();
            let mut pc = PaletteCommand::new(id.to_string(), cmd.describe(), "Ecualizador");
            if let Some(sc) = shortcut_for(&s.keymap, &cmd) {
                pc = pc.with_shortcut(sc);
            }
            catalog.push(pc);
            cmds.push(cmd);
        }
    }
    (catalog, cmds)
}

/// Reverse-lookup: el display del primer chord atado a `cmd` en el keymap.
pub(crate) fn shortcut_for(km: &media_core::control::Keymap, cmd: &MediaCommand) -> Option<String> {
    km.bindings
        .iter()
        .find(|b| &b.command == cmd)
        .map(|b| b.chord.display())
}

/// Routea un `PaletteMsg` al módulo command-palette.
pub(crate) fn apply_palette(model: crate::modelo::Model, pm: PaletteMsg, handle: &Handle<Msg>) -> crate::modelo::Model {
    let mut m = model;
    if matches!(pm, PaletteMsg::Open) && m.palette.is_none() {
        m.palette = Some(PaletteState::new(&m.palette_commands));
        return m;
    }
    let action = match m.palette.as_mut() {
        Some(state) => palette::apply(state, pm, &m.palette_commands),
        None => return m,
    };
    match action {
        PaletteAction::None => {}
        PaletteAction::Close => m.palette = None,
        PaletteAction::Invoke(id) => {
            m.palette = None;
            if let Some(cmd) = id.parse::<usize>().ok().and_then(|i| m.palette_cmds.get(i)) {
                handle.dispatch(Msg::Command(cmd.clone()));
            }
        }
    }
    m
}

/// Ejecuta un [`MediaCommand`] sobre el estado vivo del reproductor.
pub(crate) fn apply_command(cmd: MediaCommand) {
    use MediaCommand::*;
    match cmd {
        TogglePause => {
            pause().toggle();
            osd_flash(if pause().is_paused() { "Pausa" } else { "Reproduciendo" });
        }
        SeekBy { secs } => {
            seek_audio_by(secs);
            osd_flash_seek();
        }
        SeekTo { fraction } => {
            seek_audio_to(fraction);
            osd_flash_seek();
        }
        VolumeBy { delta } => {
            volume().update(|v| v + delta);
            osd_flash(osd::format_volume(volume().get()));
        }
        SetVolume { level } => {
            volume().update(|_| level);
            osd_flash(osd::format_volume(volume().get()));
        }
        ToggleMute => {
            let slot = muted_volume();
            let mut g = slot.lock();
            match g.take() {
                Some(prev) => volume().update(|_| prev),
                None => {
                    *g = Some(volume().get());
                    volume().update(|_| 0.0);
                }
            }
            drop(g);
            osd_flash(osd::format_volume(volume().get()));
        }
        PrevTrack => {
            if let Some(h) = playlist_slot().get().and_then(|o| o.as_ref()) {
                h.lock().prev();
            }
            osd_flash(media_title_string());
        }
        NextTrack => {
            if let Some(h) = playlist_slot().get().and_then(|o| o.as_ref()) {
                h.lock().next();
            }
            osd_flash(media_title_string());
        }
        ChapterNext => {
            if let Some(ch) = chapters_slot().get() {
                let pos = playback_snapshot().position;
                if let Some(c) = ch.next(pos) {
                    let title = c.title.clone();
                    seek_audio_to_pos(c.start);
                    osd_flash(format!("▸ {title}"));
                }
            }
        }
        ChapterPrev => {
            if let Some(ch) = chapters_slot().get() {
                let pos = playback_snapshot().position;
                if let Some(c) = ch.prev(pos, Duration::from_secs(3)) {
                    let title = c.title.clone();
                    seek_audio_to_pos(c.start);
                    osd_flash(format!("◂ {title}"));
                }
            }
        }
        SpeedStep { dir } => {
            step_speed(dir);
            osd_flash(osd::format_speed(player_speed() as f32));
        }
        SetSpeed { mult } => {
            set_speed_abs(mult);
            osd_flash(osd::format_speed(player_speed() as f32));
        }
        FrameStep { dir } => {
            // El stepping siempre deja el reproductor en pausa.
            pause().pause();
            if dir < 0 {
                // Hacia atrás: reposicioná un intervalo de cuadro antes. En la
                // ruta ffmpeg el seek respawnea la sesión compartida y mueve
                // también el video; en fuentes nativas (AV1/GIF) sólo mueve el
                // audio (aproximación de MVP). El destino se pinta vía el
                // step_frame que dispara FRAME_STEP_FWD tras el respawn.
                let frame_dt = Duration::from_secs_f64(1.0 / crate::estado::video_fps() as f64);
                let pos = playback_snapshot().position;
                seek_audio_to_pos(pos.saturating_sub(frame_dt));
            }
            crate::estado::FRAME_STEP_FWD.store(true, Ordering::Relaxed);
            osd_flash(if dir < 0 { "◂ Cuadro" } else { "Cuadro ▸" });
        }
        CycleRepeat => {
            if let Some(h) = playlist_slot().get().and_then(|o| o.as_ref()) {
                let mut pl = h.lock();
                pl.cycle_repeat();
                let mode = pl.repeat_mode();
                eprintln!("media-app: repeat {}", mode.label());
                drop(pl);
                osd_flash(match mode {
                    RepeatMode::Off => "Repetir: no",
                    RepeatMode::One => "Repetir: una",
                    RepeatMode::All => "Repetir: todo",
                });
            }
        }
        ToggleShuffle => {
            if let Some(h) = playlist_slot().get().and_then(|o| o.as_ref()) {
                let mut pl = h.lock();
                pl.toggle_shuffle();
                let on = pl.shuffle_on();
                eprintln!("media-app: shuffle {}", if on { "on" } else { "off" });
                drop(pl);
                osd_flash(if on { "Aleatorio: sí" } else { "Aleatorio: no" });
            }
        }
        Snapshot => do_snapshot(),
        ToggleRecord => toggle_record(),
        Script { name } => run_script(&name),
        EqToggle => {
            let e = eq();
            let on = !e.is_enabled();
            e.set_enabled(on);
            eprintln!("media-app: eq {}", if on { "on" } else { "off" });
        }
        EqBandBy { idx, delta_db } => {
            let e = eq();
            let cur = e.gains().get(idx).copied().unwrap_or(0.0);
            e.set_gain(idx, (cur + delta_db).clamp(-12.0, 12.0));
        }
        EqReset => {
            eq().set_all_gains(&[0.0; ISO_10_BANDS_HZ.len()]);
            eprintln!("media-app: eq plano");
        }
        AvSyncBy { ms } => {
            if let Some(pipe) = pipeline_slot().get() {
                let mut s = pipe.sync.lock();
                s.add_offset_ms(ms);
                let off = s.offset_ms();
                drop(s);
                eprintln!("media-app: sync A/V {off:+}ms");
                osd_flash(format!("Sync A/V {off:+} ms"));
            }
        }
        AvSyncReset => {
            if let Some(pipe) = pipeline_slot().get() {
                pipe.sync.lock().set_offset_ms(0);
                eprintln!("media-app: sync A/V a cero");
            }
        }
        ColorToggle => {
            let c = color();
            let on = !c.is_enabled();
            c.set_enabled(on);
            eprintln!("media-app: color {}", if on { "on" } else { "off" });
        }
        ColorBy { param, delta } => {
            let c = color();
            match param {
                ColorParam::Brightness => c.add_brightness(delta),
                ColorParam::Contrast => c.add_contrast(delta),
                ColorParam::Gamma => c.add_gamma(delta),
                ColorParam::Saturation => c.add_saturation(delta),
                ColorParam::Hue => c.add_hue(delta),
            }
        }
        ColorReset => {
            color().reset();
            eprintln!("media-app: color original");
        }
        RotateBy { dir } => {
            transform().rotate(dir);
            eprintln!("media-app: rotación {}", transform().transform().rotation.label());
        }
        FlipH => transform().toggle_flip_h(),
        FlipV => transform().toggle_flip_v(),
        OrientReset => {
            transform().reset();
            eprintln!("media-app: orientación original");
        }
        SubDelayBy { ms } => {
            let new = (SUB_DELAY_MS.load(Ordering::Relaxed) + ms)
                .clamp(-MAX_SUB_DELAY_MS, MAX_SUB_DELAY_MS);
            SUB_DELAY_MS.store(new, Ordering::Relaxed);
            eprintln!("media-app: subtítulo {new:+}ms");
            osd_flash(format!("Subtítulo {new:+} ms"));
        }
        SubDelayReset => {
            SUB_DELAY_MS.store(0, Ordering::Relaxed);
            eprintln!("media-app: subtítulo sin delay");
        }
        NormToggle => {
            let d = dynamics();
            let on = !d.is_enabled();
            d.set_enabled(on);
            eprintln!("media-app: normalización {}", if on { "on" } else { "off" });
        }
        NormGainBy { db } => {
            let d = dynamics();
            d.add_gain_db(db);
            eprintln!("media-app: normalización {:+.0} dB", d.gain_db());
        }
        NormReset => {
            dynamics().reset();
            eprintln!("media-app: normalización a 0 dB");
        }
        NormAuto => {
            match loudness().gain_to_target_db(REPLAYGAIN_TARGET_LUFS) {
                Some(gain) => {
                    let d = dynamics();
                    d.set_enabled(true);
                    d.set_gain_db(gain);
                    eprintln!(
                        "media-app: normalización automática → {:+.1} dB (objetivo {:.0} LUFS)",
                        d.gain_db(),
                        REPLAYGAIN_TARGET_LUFS
                    );
                }
                None => eprintln!(
                    "media-app: normalización automática — aún sin medición \
                     (reproducí ≳ 1 s primero)"
                ),
            }
        }
        BookmarkToggle => {
            let Some(key) = current_track_key() else {
                osd_flash("Marcas: sólo en archivos");
                return;
            };
            let pos = playback_snapshot().position;
            let eps = Duration::from_millis(750);
            let mut bm = crate::config_io::bookmarks().lock();
            if bm.remove_near(&key, pos, eps) {
                drop(bm);
                osd_flash("Marca quitada");
            } else {
                bm.add(&key, pos, "");
                drop(bm);
                osd_flash(format!("Marca · {}", osd::format_hms(pos.as_secs_f64())));
            }
            crate::config_io::save_bookmarks();
        }
        BookmarkNext => {
            if let Some(key) = current_track_key() {
                let pos = playback_snapshot().position;
                if let Some(m) = crate::config_io::bookmarks().lock().next_after(&key, pos).cloned() {
                    seek_audio_to_pos(m.position);
                    osd_flash(format!("▸ Marca {}", osd::format_hms(m.position.as_secs_f64())));
                }
            }
        }
        BookmarkPrev => {
            if let Some(key) = current_track_key() {
                let pos = playback_snapshot().position;
                if let Some(m) = crate::config_io::bookmarks().lock().prev_before(&key, pos).cloned() {
                    seek_audio_to_pos(m.position);
                    osd_flash(format!("◂ Marca {}", osd::format_hms(m.position.as_secs_f64())));
                }
            }
        }
        ViewCycleFit => {
            let mut v = viewcontrol().lock();
            v.cycle_fit();
            let label = v.fit.label();
            drop(v);
            osd_flash(format!("Encaje: {label}"));
        }
        ViewZoomBy { factor } => {
            let mut v = viewcontrol().lock();
            v.zoom_by(factor);
            let z = v.zoom;
            drop(v);
            osd_flash(format!("Zoom {z:.2}×"));
        }
        ViewPanBy { dx, dy } => {
            viewcontrol().lock().pan_by(dx, dy);
        }
        ViewReset => {
            viewcontrol().lock().reset();
            osd_flash("Vista original");
        }
        CycleAudioTrack => {
            let Some(session) = ffmpeg_session_slot().get().and_then(|o| o.as_ref()) else {
                osd_flash("Sin pistas de audio");
                return;
            };
            let mut guard = tracks().lock();
            let Some(ts) = guard.as_mut() else {
                drop(guard);
                osd_flash("Sin pistas de audio");
                return;
            };
            if !ts.has_audio_choice() {
                drop(guard);
                osd_flash("Una sola pista de audio");
                return;
            }
            let picked = ts.cycle_audio().map(|t| (t.index, t.label()));
            drop(guard);
            if let Some((index, label)) = picked {
                let from = playback_snapshot().position;
                match session.select_audio_stream(index, from) {
                    Ok(()) => {
                        crate::estado::reset_av_sync_anchor();
                        osd_flash(format!("Audio: {label}"));
                    }
                    Err(e) => eprintln!("media-app: cambio de pista de audio: {e}"),
                }
            }
        }
        CycleSubtitleTrack => {
            let mut guard = tracks().lock();
            let Some(ts) = guard.as_mut().filter(|t| t.has_subtitles()) else {
                drop(guard);
                osd_flash("Sin subtítulos embebidos");
                return;
            };
            ts.cycle_subtitle();
            let sel = ts.current_subtitle().map(|t| (t.index, t.label()));
            drop(guard);
            match sel {
                None => {
                    *subtitles_slot().lock() = None;
                    osd_flash("Subtítulos: apagado");
                }
                Some((index, label)) => {
                    let Some(path) = video_path_slot().get().cloned() else {
                        osd_flash("Subtítulos: sin archivo");
                        return;
                    };
                    osd_flash(format!("Subtítulos: {label}…"));
                    std::thread::spawn(move || {
                        match foreign_av::extract_subtitle(&path, index) {
                            Ok(text) => match media_core::SubtitleTrack::parse_subtitles(&text) {
                                Ok(t) => {
                                    *subtitles_slot().lock() = Some(t);
                                    osd().lock().flash(format!("Subtítulos: {label}"), osd_now());
                                }
                                Err(e) => {
                                    eprintln!("media-app: subtítulo embebido ilegible: {e}");
                                    osd().lock().flash("Subtítulo ilegible", osd_now());
                                }
                            },
                            Err(e) => {
                                eprintln!("media-app: extracción de subtítulo: {e}");
                                osd().lock().flash("Subtítulo no es de texto", osd_now());
                            }
                        }
                    });
                }
            }
        }
    }
}

/// Ejecuta el script Rhai `name` de la biblioteca de `settings()`.
pub(crate) fn run_script(name: &str) {
    let Some(src) = crate::estado::settings().script(name).map(str::to_string) else {
        eprintln!("media-app: script «{name}» no existe en controles.ron");
        return;
    };
    let engine = script_engine();
    if let Err(e) = engine.run(&src) {
        eprintln!("media-app: script «{name}»: {e}");
    }
}

/// Velocidad de reproducción actual (1.0× si no hay playlist).
pub(crate) fn player_speed() -> f64 {
    playlist_slot()
        .get()
        .and_then(|o| o.as_ref())
        .map(|h| h.lock().current_speed() as f64)
        .unwrap_or(1.0)
}

/// Arma un motor Rhai con la API del reproductor bindeada.
pub(crate) fn script_engine() -> rhai::Engine {
    let mut engine = rhai::Engine::new();
    engine.set_max_operations(50_000);
    engine.register_fn("toggle_pause", || {
        pause().toggle();
    });
    engine.register_fn("pause", || pause().pause());
    engine.register_fn("resume", || pause().resume());
    engine.register_fn("is_paused", || pause().is_paused());
    engine.register_fn("seek", |secs: i64| seek_audio_by(secs));
    engine.register_fn("volume", || volume().get() as f64);
    engine.register_fn("set_volume", |level: f64| {
        volume().update(|_| level as f32);
    });
    engine.register_fn("add_volume", |delta: f64| {
        volume().update(|v| v + delta as f32);
    });
    engine.register_fn("speed", player_speed);
    engine.register_fn("set_speed", |mult: f64| set_speed_abs(mult as f32));
    engine.register_fn("step_speed", |dir: i64| step_speed(dir as i32));
    engine.register_fn("next_track", || apply_command(MediaCommand::NextTrack));
    engine.register_fn("prev_track", || apply_command(MediaCommand::PrevTrack));
    engine.register_fn("cycle_repeat", || apply_command(MediaCommand::CycleRepeat));
    engine.register_fn("toggle_shuffle", || {
        apply_command(MediaCommand::ToggleShuffle)
    });
    engine.register_fn("snapshot", do_snapshot);
    engine.register_fn("toggle_record", toggle_record);
    engine.register_fn("is_recording", || recorder().is_recording());
    engine
}

/// Cicla la velocidad `dir` pasos por `settings().speed_steps`.
pub(crate) fn step_speed(dir: i32) {
    let Some(handle) = playlist_slot().get().and_then(|o| o.as_ref()) else {
        return;
    };
    let s = crate::estado::settings();
    let steps = &s.speed_steps;
    if steps.is_empty() {
        return;
    }
    let mut pl = handle.lock();
    let cur = pl.current_speed();
    let idx = steps
        .iter()
        .position(|&s| (s - cur).abs() < 1e-3)
        .unwrap_or(0) as i32;
    let n = steps.len() as i32;
    let next_idx = ((idx + dir) % n + n) % n;
    let next = steps[next_idx as usize];
    pl.set_speed(next);
    eprintln!("media-app: speed {:.2}×", next);
}

/// Fija una velocidad absoluta.
pub(crate) fn set_speed_abs(mult: f32) {
    if let Some(handle) = playlist_slot().get().and_then(|o| o.as_ref()) {
        handle.lock().set_speed(mult);
        eprintln!("media-app: speed {:.2}×", mult);
    }
}

/// Arma/cierra la grabación WAV del stream de audio en el cwd.
pub(crate) fn toggle_record() {
    let rec = recorder();
    if rec.is_recording() {
        match rec.stop() {
            Ok(p) => eprintln!("media-app: recording cerrada en {}", p.display()),
            Err(e) => eprintln!("media-app: stop recording: {e}"),
        }
    } else {
        let path = default_recording_path(".");
        match rec.start(&path) {
            Ok(p) => eprintln!("media-app: grabando en {}", p.display()),
            Err(e) => eprintln!("media-app: start recording: {e}"),
        }
    }
}

/// Escribe un PNG con el frame de video pendiente.
pub(crate) fn do_snapshot() {
    let Some(pipe) = pipeline_slot().get() else {
        eprintln!("media-app: pipeline aún no montada");
        return;
    };
    let (w, h) = *pipe.last_dim.lock();
    let buf = pipe.buf.lock().clone();
    let expected = (w as usize) * (h as usize) * 4;
    if w == 0 || h == 0 || buf.len() != expected {
        eprintln!("media-app: no hay frame para snapshot todavía");
        return;
    }
    let path = default_snapshot_path();
    match image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(w, h, buf) {
        Some(img) => match img.save(&path) {
            Ok(()) => eprintln!(
                "media-app: snapshot {}×{} guardado en {}",
                w,
                h,
                path.display()
            ),
            Err(e) => eprintln!("media-app: save snapshot: {e}"),
        },
        None => eprintln!("media-app: buf inconsistente para snapshot"),
    }
}

/// Path de snapshot único por segundo.
pub(crate) fn default_snapshot_path() -> std::path::PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    std::path::PathBuf::from(format!("media-snap-{secs}.png"))
}

/// Traduce un evento de teclado de Llimphi al [`KeyChord`] canónico.
pub(crate) fn chord_from_event(ev: &KeyEvent) -> Option<KeyChord> {
    if ev.state != KeyState::Pressed {
        return None;
    }
    let key = match &ev.key {
        Key::Named(NamedKey::Space) => "Space".to_string(),
        Key::Named(NamedKey::ArrowLeft) => "ArrowLeft".to_string(),
        Key::Named(NamedKey::ArrowRight) => "ArrowRight".to_string(),
        Key::Named(NamedKey::ArrowUp) => "ArrowUp".to_string(),
        Key::Named(NamedKey::ArrowDown) => "ArrowDown".to_string(),
        Key::Named(NamedKey::Enter) => "Enter".to_string(),
        Key::Character(c) => c.to_lowercase(),
        _ => return None,
    };
    Some(KeyChord {
        key,
        ctrl: ev.modifiers.ctrl,
        shift: ev.modifiers.shift,
        alt: ev.modifiers.alt,
    })
}
