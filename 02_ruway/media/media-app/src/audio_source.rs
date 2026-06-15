use std::path::PathBuf;
use std::sync::Arc;

use media_core::{
    AudioSource, MixerAudio, PausableAudio, ProbedAudioSource, ToneSource, Volume, VolumeAudio,
};
use media_core::dynamics::DynamicsAudio;
use media_core::eq::EqualizerAudio;
use media_core::loudness::LoudnessProbe;
use media_core::AudioProbe;
use media_recorder_wav::RecordedAudioSource;
use foreign_av::FfmpegAudioSource;
use parking_lot::Mutex;

use crate::estado::{
    dynamics, eq, ffmpeg_session_slot, loudness, pause, playlist_labels_slot, playlist_slot,
    recorder, video_path_slot, volume, PROBE_CAPACITY,
};
use crate::media_io::load_playlist_file;
use crate::playlist::{LoadedTrack, Playlist, SharedAudio};

pub(crate) fn audio_source_from_env() -> (Arc<Mutex<dyn AudioSource + Send>>, AudioProbe) {
    let probe = AudioProbe::new(PROBE_CAPACITY);

    // Prioridad 0: si hay session ffmpeg (modo video file), el audio
    // sale de ahí — mismo proceso que el video.
    if let Some(Some(session)) = ffmpeg_session_slot().get() {
        match FfmpegAudioSource::from_session(session.clone()) {
            Ok(audio) => {
                eprintln!(
                    "media-app: ffmpeg audio @ {} Hz · {} ch",
                    audio.source_sample_rate(),
                    audio.source_channels(),
                );
                let label = video_path_slot()
                    .get()
                    .cloned()
                    .unwrap_or_else(|| PathBuf::from("video"));
                let pl = Playlist::new_single(label, LoadedTrack::FfmpegAudio(audio));
                *playlist_labels_slot().lock() = pl.track_labels();
                let shared: Arc<Mutex<Playlist>> = Arc::new(Mutex::new(pl));
                playlist_slot().set(Some(shared.clone())).ok();
                let pausable = PausableAudio::new(
                    Box::new(SharedAudio { inner: shared })
                        as Box<dyn AudioSource + Send>,
                    pause().clone(),
                );
                let voled = VolumeAudio::new(pausable, volume().clone());
                let equalized = EqualizerAudio::new(voled, eq().clone());
                let measured = LoudnessProbe::new(equalized, loudness().clone());
                let normalized = DynamicsAudio::new(measured, dynamics().clone());
                let recorded = RecordedAudioSource::new(normalized, recorder().clone());
                let probed = ProbedAudioSource::new(recorded, probe.clone());
                return (Arc::new(Mutex::new(probed)), probe);
            }
            Err(e) => {
                eprintln!(
                    "media-app: ffmpeg audio falló ({e}) — sigo sin track audio"
                );
            }
        }
    }

    let tracks: Option<Vec<PathBuf>> =
        if let Ok(playlist_path) = std::env::var("MEDIA_PLAYLIST") {
            match load_playlist_file(&playlist_path) {
                Ok(t) if !t.is_empty() => Some(t),
                Ok(_) => {
                    eprintln!("media-app: playlist {playlist_path} vacía");
                    None
                }
                Err(e) => {
                    eprintln!("media-app: no pude leer playlist {playlist_path}: {e}");
                    None
                }
            }
        } else if let Ok(p) = std::env::var("MEDIA_WAV") {
            Some(vec![PathBuf::from(p)])
        } else if let Ok(p) = std::env::var("MEDIA_MP3") {
            Some(vec![PathBuf::from(p)])
        } else {
            None
        };

    let inner: Box<dyn AudioSource + Send> = if let Some(tracks) = tracks {
        match Playlist::new(tracks) {
            Ok(pl) => {
                eprintln!(
                    "media-app: playlist [1/{}] → {}",
                    pl.len(),
                    pl.track_path().display(),
                );
                *playlist_labels_slot().lock() = pl.track_labels();
                let shared: Arc<Mutex<Playlist>> = Arc::new(Mutex::new(pl));
                playlist_slot().set(Some(shared.clone())).ok();
                Box::new(SharedAudio { inner: shared })
            }
            Err(e) => {
                eprintln!("media-app: playlist falló ({e}) — motor vacío (silencio)");
                let shared: Arc<Mutex<Playlist>> = Arc::new(Mutex::new(Playlist::empty()));
                playlist_slot().set(Some(shared.clone())).ok();
                Box::new(SharedAudio { inner: shared })
            }
        }
    } else {
        // Sin medio inicial: motor vivo en silencio, listo para que una
        // playlist se cargue en caliente (perfiles / Cola).
        let shared: Arc<Mutex<Playlist>> = Arc::new(Mutex::new(Playlist::empty()));
        playlist_slot().set(Some(shared.clone())).ok();
        Box::new(SharedAudio { inner: shared })
    };

    let inner: Box<dyn AudioSource + Send> = match std::env::var("MEDIA_MIX_TONE")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
    {
        Some(g) if g > 0.0 => {
            let g = g.min(1.0);
            eprintln!("media-app: overlay tono A4 a {:.0}%", g * 100.0);
            let tone = VolumeAudio::new(ToneSource::a4(), Volume::new(g));
            let mix = MixerAudio::with_sources(vec![inner, Box::new(tone)]);
            Box::new(mix)
        }
        _ => inner,
    };
    let pausable = PausableAudio::new(inner, pause().clone());
    let voled = VolumeAudio::new(pausable, volume().clone());
    let equalized = EqualizerAudio::new(voled, eq().clone());
    let measured = LoudnessProbe::new(equalized, loudness().clone());
    let normalized = DynamicsAudio::new(measured, dynamics().clone());
    let recorded = RecordedAudioSource::new(normalized, recorder().clone());
    let probed = ProbedAudioSource::new(recorded, probe.clone());
    (Arc::new(Mutex::new(probed)), probe)
}
