use std::collections::HashMap;

use transmux::{CodecConfig, Media};

const NAL_LENGTH_BYTES: usize = 4;
const NAL_TYPE_MASK: u8 = 0x1f;
const NAL_SLICE: u8 = 1;
const NAL_IDR: u8 = 5;

pub struct GroundTruth {
    pub audio_matched: usize,
    pub video_matched: usize,
    pub audio_median: f64,
    pub video_median: f64,
}

impl GroundTruth {
    pub fn offset(&self) -> f64 {
        self.audio_median - self.video_median
    }
}

#[derive(Default)]
pub struct Truth {
    video: HashMap<Vec<u8>, Option<f64>>,
    audio: HashMap<Vec<u8>, Option<f64>>,
    first_video: Option<f64>,
    first_audio: Option<f64>,
}

fn video_key(data: &[u8]) -> Vec<u8> {
    let mut key = Vec::new();
    let mut at = 0;
    while at + NAL_LENGTH_BYTES <= data.len() {
        let len =
            u32::from_be_bytes(data[at..at + NAL_LENGTH_BYTES].try_into().expect("len")) as usize;
        at += NAL_LENGTH_BYTES;
        let Some(nal) = data.get(at..at + len) else {
            break;
        };
        if matches!(
            nal.first().map(|b| b & NAL_TYPE_MASK),
            Some(NAL_SLICE | NAL_IDR)
        ) {
            key.extend_from_slice(nal);
        }
        at += len;
    }
    key
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

impl Truth {
    pub fn from_media(media: &Media) -> Self {
        let mut truth = Self::default();
        for track in &media.tracks {
            let video = matches!(track.spec.config, CodecConfig::Avc { .. });
            let scale = f64::from(track.spec.timescale);
            for s in &track.samples {
                let Some(pts) = s.pts else { continue };
                let key = if video {
                    video_key(&s.data)
                } else {
                    s.data.to_vec()
                };
                let first = if video {
                    &mut truth.first_video
                } else {
                    &mut truth.first_audio
                };
                first.get_or_insert(pts as f64 / scale);
                let map = if video {
                    &mut truth.video
                } else {
                    &mut truth.audio
                };
                map.entry(key)
                    .and_modify(|v| *v = None)
                    .or_insert(Some(pts as f64 / scale));
            }
        }
        truth
    }

    pub fn source_start_gap(&self) -> f64 {
        self.first_audio.unwrap_or(0.0) - self.first_video.unwrap_or(0.0)
    }

    pub fn measure<'a>(&self, samples: impl Iterator<Item = (bool, &'a [u8], f64)>) -> GroundTruth {
        let (mut audio, mut video) = (Vec::new(), Vec::new());
        for (is_video, data, pts) in samples {
            let (map, key, out) = if is_video {
                (&self.video, video_key(data), &mut video)
            } else {
                (&self.audio, data.to_vec(), &mut audio)
            };
            if let Some(Some(source)) = map.get(&key) {
                out.push(pts - source);
            }
        }
        assert!(
            !audio.is_empty() && !video.is_empty(),
            "no emitted sample matched a source access unit (audio {}, video {})",
            audio.len(),
            video.len()
        );
        GroundTruth {
            audio_matched: audio.len(),
            video_matched: video.len(),
            audio_median: median(&mut audio),
            video_median: median(&mut video),
        }
    }
}
