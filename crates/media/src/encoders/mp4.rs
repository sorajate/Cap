use crate::{
    data::{FFAudio, FFVideo, RawVideoFormat},
    pipeline::task::PipelineSinkTask,
    MediaError,
};
use ffmpeg::format::{self};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tracing::{info, trace};

use super::{audio::AudioEncoder, H264Encoder};

pub struct MP4File {
    tag: &'static str,
    output: format::context::Output,
    video: H264Encoder,
    audio: Option<Box<dyn AudioEncoder + Send>>,
    is_finished: bool,
}

#[derive(thiserror::Error, Debug)]
pub enum InitError {
    #[error("ffmpeg error: {0}")]
    Ffmpeg(ffmpeg::Error),
    #[error("video init: {0}")]
    VideoInit(MediaError),
    #[error("audio init: {0}")]
    AudioInit(MediaError),
}

impl From<InitError> for MediaError {
    fn from(value: InitError) -> Self {
        match value {
            InitError::AudioInit(e) | InitError::VideoInit(e) => e,
            InitError::Ffmpeg(e) => Self::FFmpeg(e),
        }
    }
}

impl MP4File {
    pub fn init(
        tag: &'static str,
        mut output: PathBuf,
        video: impl FnOnce(&mut format::context::Output) -> Result<H264Encoder, MediaError>,
        audio: impl FnOnce(
            &mut format::context::Output,
        ) -> Option<Result<Box<dyn AudioEncoder + Send>, MediaError>>,
    ) -> Result<Self, InitError> {
        output.set_extension("mp4");

        if let Some(parent) = output.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let mut output = format::output(&output).map_err(InitError::Ffmpeg)?;

        trace!("Preparing encoders for mp4 file");

        let video = video(&mut output).map_err(InitError::VideoInit)?;
        let audio = audio(&mut output)
            .transpose()
            .map_err(InitError::AudioInit)?;

        info!("Prepared encoders for mp4 file");

        // make sure this happens after adding all encoders!
        output.write_header().map_err(InitError::Ffmpeg)?;

        Ok(Self {
            tag,
            output,
            video,
            audio,
            is_finished: false,
        })
    }

    pub fn video_format() -> RawVideoFormat {
        RawVideoFormat::YUYV420
    }

    pub fn queue_video_frame(&mut self, frame: FFVideo) {
        if self.is_finished {
            return;
        }

        self.video.queue_frame(frame, &mut self.output);
    }

    pub fn queue_audio_frame(&mut self, frame: FFAudio) {
        if self.is_finished {
            return;
        }

        let Some(audio) = &mut self.audio else {
            return;
        };

        audio.queue_frame(frame, &mut self.output);
    }

    pub fn finish(&mut self) {
        if self.is_finished {
            return;
        }

        self.is_finished = true;

        tracing::info!("MP4Encoder: Finishing encoding");

        self.video.finish(&mut self.output);

        if let Some(audio) = &mut self.audio {
            tracing::info!("MP4Encoder: Flushing audio encoder");
            audio.finish(&mut self.output);
        }

        tracing::info!("MP4Encoder: Writing trailer");
        if let Err(e) = self.output.write_trailer() {
            tracing::error!("Failed to write MP4 trailer: {:?}", e);
        }
    }
}

pub struct MP4Input {
    pub video: FFVideo,
    pub audio: Option<FFAudio>,
}

unsafe impl Send for H264Encoder {}

impl PipelineSinkTask<MP4Input> for MP4File {
    fn run(
        &mut self,
        ready_signal: crate::pipeline::task::PipelineReadySignal,
        input: &flume::Receiver<MP4Input>,
    ) {
        ready_signal.send(Ok(())).unwrap();

        while let Ok(frame) = input.recv() {
            self.queue_video_frame(frame.video);
            if let Some(audio) = frame.audio {
                self.queue_audio_frame(audio);
            }
        }
    }

    fn finish(&mut self) {
        self.finish();
    }
}

impl PipelineSinkTask<FFVideo> for MP4File {
    fn run(
        &mut self,
        ready_signal: crate::pipeline::task::PipelineReadySignal,
        input: &flume::Receiver<FFVideo>,
    ) {
        assert!(self.audio.is_none());

        ready_signal.send(Ok(())).unwrap();

        while let Ok(frame) = input.recv() {
            self.queue_video_frame(frame);
        }
    }

    fn finish(&mut self) {
        self.finish();
    }
}

impl PipelineSinkTask<FFAudio> for Arc<Mutex<MP4File>> {
    fn run(
        &mut self,
        ready_signal: crate::pipeline::task::PipelineReadySignal,
        input: &flume::Receiver<FFAudio>,
    ) {
        ready_signal.send(Ok(())).ok();

        while let Ok(frame) = input.recv() {
            let mut this = self.lock().unwrap();
            this.queue_audio_frame(frame);
        }
    }

    fn finish(&mut self) {
        self.lock().unwrap().finish();
    }
}

impl PipelineSinkTask<FFVideo> for Arc<Mutex<MP4File>> {
    fn run(
        &mut self,
        ready_signal: crate::pipeline::task::PipelineReadySignal,
        input: &flume::Receiver<FFVideo>,
    ) {
        ready_signal.send(Ok(())).ok();

        while let Ok(frame) = input.recv() {
            let mut this = self.lock().unwrap();
            this.queue_video_frame(frame);
        }
    }

    fn finish(&mut self) {
        self.lock().unwrap().finish();
    }
}
