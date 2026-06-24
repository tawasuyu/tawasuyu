use std::time::Instant;

use llimphi_surface::ExternalSurface;
use llimphi_ui::llimphi_hal::wgpu;
use media_core::{FrameSource, TestCard};
use media_core::color::ColorVideo;
use media_core::sync::AvSync;
use media_core::transform::TransformVideo;
use media_source_gif::GifSource;
use media_source_image::ImageSource;
use foreign_av::FfmpegVideoSource;
use parking_lot::Mutex;

use crate::estado::{
    color, config_slot, ffmpeg_session_slot, pipeline_slot, set_video_fps, transform,
    video_path_slot, TESTCARD_W, TESTCARD_H, TESTCARD_FPS,
};
use crate::tipos::{Msg, VideoKind};

pub(crate) struct Pipeline {
    pub(crate) surface: ExternalSurface,
    pub(crate) source: Mutex<Box<dyn FrameSource + Send>>,
    pub(crate) buf: Mutex<Vec<u8>>,
    pub(crate) last_dim: Mutex<(u32, u32)>,
    pub(crate) last_tick: Mutex<Instant>,
    pub(crate) sync: Mutex<AvSync>,
}

pub(crate) fn new_testcard() -> Box<dyn FrameSource + Send> {
    Box::new(TestCard::new(TESTCARD_W, TESTCARD_H, TESTCARD_FPS))
}

pub(crate) fn build_video_source() -> Box<dyn FrameSource + Send> {
    let cfg = config_slot().get().expect("config set");
    match cfg.kind {
        VideoKind::Testcard => {
            set_video_fps(TESTCARD_FPS);
            new_testcard()
        }
        VideoKind::Gif => {
            let path = video_path_slot().get().expect("video path set");
            match GifSource::from_path(path) {
                Ok(s) => Box::new(s),
                Err(e) => {
                    eprintln!(
                        "media-app: error abriendo GIF {path:?}: {e} — caigo a testcard"
                    );
                    new_testcard()
                }
            }
        }
        VideoKind::Image => {
            let path = video_path_slot().get().expect("video path set");
            match ImageSource::from_path(path) {
                Ok(s) => Box::new(s),
                Err(e) => {
                    eprintln!(
                        "media-app: error abriendo imagen {path:?}: {e} — caigo a testcard"
                    );
                    new_testcard()
                }
            }
        }
        VideoKind::Ffmpeg => {
            match ffmpeg_session_slot()
                .get()
                .and_then(|o| o.as_ref())
                .ok_or_else(|| "ffmpeg session no disponible".to_string())
                .and_then(|s| {
                    FfmpegVideoSource::from_session(s.clone())
                        .map_err(|e| e.to_string())
                }) {
                Ok(s) => {
                    set_video_fps(s.fps());
                    Box::new(s)
                }
                Err(e) => {
                    eprintln!("media-app: ffmpeg video: {e} — caigo a testcard");
                    new_testcard()
                }
            }
        }
        VideoKind::Av1 => {
            let path = video_path_slot().get().expect("video path set");
            match media_source_av1::Av1VideoSource::open(path) {
                Ok(s) => {
                    set_video_fps(s.fps());
                    Box::new(s)
                }
                Err(e) => {
                    eprintln!("media-app: AV1 nativo {path:?}: {e} — caigo a testcard");
                    new_testcard()
                }
            }
        }
    }
}

pub(crate) fn pipeline_for(device: &wgpu::Device, queue: &wgpu::Queue) -> &'static Pipeline {
    pipeline_slot().get_or_init(|| Pipeline {
        surface: ExternalSurface::new(device, queue),
        source: Mutex::new(Box::new(TransformVideo::new(
            ColorVideo::new(build_video_source(), color().clone()),
            transform().clone(),
        ))),
        buf: Mutex::new(Vec::new()),
        last_dim: Mutex::new((0, 0)),
        last_tick: Mutex::new(Instant::now()),
        sync: Mutex::new(AvSync::default()),
    })
}

/// Conecta el cliente del rail hospedado: por default delega a pata cuando está
/// corriendo (opt-out con `MEDIA_DELEGATE_SIDEBAR=0`).
pub(crate) fn media_host(handle: &llimphi_ui::Handle<Msg>) -> Option<pata_host::HostClient> {
    if !pata_host::delegate_sidebar_default("MEDIA_DELEGATE_SIDEBAR") {
        return None;
    }
    let teeth = vec![
        pata_host::HostedTooth::new(0, "settings", "Config"),
        pata_host::HostedTooth::new(1, "files", "Cola"),
        pata_host::HostedTooth::new(2, "astro", "Visualizadores"),
        pata_host::HostedTooth::new(3, "tools", "Ayuda"),
    ];
    let h = handle.clone();
    pata_host::HostClient::connect("tawasuyu.media", "Media", teeth, move |id| {
        h.dispatch(Msg::HostActivate(id))
    })
}
