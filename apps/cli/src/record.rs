use std::{env::current_dir, path::PathBuf, sync::Arc};

use cap_media::{feeds::CameraFeed, sources::ScreenCaptureTarget};
use clap::Args;
use nokhwa::utils::{ApiBackend, CameraIndex};
use tokio::{io::AsyncBufReadExt, sync::Mutex};
use uuid::Uuid;

#[derive(Args)]
pub struct RecordStart {
    #[command(flatten)]
    target: RecordTargets,
    /// Index of the camera to record
    #[arg(long)]
    camera: Option<u32>,
    /// ID of the microphone to record
    #[arg(long)]
    mic: Option<u32>,
    /// Whether to capture system audio
    #[arg(long)]
    system_audio: bool,
    /// Path to save the '.cap' project to
    #[arg(long)]
    path: Option<PathBuf>,
    /// Maximum fps to record at (max 60)
    #[arg(long)]
    fps: Option<u32>,
}

impl RecordStart {
    pub async fn run(self) -> Result<(), String> {
        let (target_info, _) = match (self.target.screen, self.target.window) {
            (Some(id), _) => cap_media::sources::list_screens()
                .into_iter()
                .find(|s| s.0.id == id)
                .map(|(s, t)| (ScreenCaptureTarget::Screen { id: s.id }, t))
                .ok_or(format!("Screen with id '{id}' not found")),
            (_, Some(id)) => cap_media::sources::list_windows()
                .into_iter()
                .find(|s| s.0.id == id)
                .map(|(s, t)| (ScreenCaptureTarget::Window { id: s.id }, t))
                .ok_or(format!("Window with id '{id}' not found")),
            _ => Err("No target specified".to_string()),
        }?;

        let camera = if let Some(camera_index) = self.camera {
            if let Some(camera_info) = nokhwa::query(ApiBackend::Auto)
                .unwrap()
                .into_iter()
                .find(|c| *c.index() == CameraIndex::Index(camera_index))
            {
                let name = camera_info.human_name();

                Some(CameraFeed::init(&name).await.unwrap())
            } else {
                None
            }
        } else {
            None
        };

        let id = Uuid::new_v4().to_string();
        let path = self
            .path
            .unwrap_or_else(|| current_dir().unwrap().join(format!("{id}.cap")));

        let actor = cap_recording::spawn_studio_recording_actor(
            id,
            path,
            cap_recording::RecordingBaseInputs {
                capture_target: target_info,
                capture_system_audio: self.system_audio,
                mic_feed: &None,
            },
            camera.map(|c| Arc::new(Mutex::new(c))),
            false,
        )
        .await
        .map_err(|e| e.to_string())?;

        println!("Recording starting, press Enter to stop");

        tokio::io::BufReader::new(tokio::io::stdin())
            .read_line(&mut String::new())
            .await
            .unwrap();

        actor.0.stop().await.unwrap();

        Ok(())
    }
}

#[derive(Args)]
struct RecordTargets {
    /// ID of the screen to capture
    #[arg(long, group = "target")]
    screen: Option<u32>,
    /// ID of the window to capture
    #[arg(long, group = "target")]
    window: Option<u32>,
}
