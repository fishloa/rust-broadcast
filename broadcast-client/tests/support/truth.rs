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
    video: Vec<(Vec<u8>, f64)>,
    audio: Vec<(Vec<u8>, f64)>,
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

fn candidates(source: &[(Vec<u8>, f64)], emitted: &[(Vec<u8>, f64)]) -> (usize, Vec<f64>) {
    let mut best = 0;
    let mut found: Vec<(usize, f64)> = Vec::new();
    for shift in 0..source.len() {
        let mut deltas = Vec::new();
        for (i, (key, pts)) in emitted.iter().enumerate() {
            if let Some((skey, spts)) = source.get(shift + i)
                && skey == key
            {
                deltas.push(pts - spts);
            }
        }
        if deltas.is_empty() {
            continue;
        }
        best = best.max(deltas.len());
        found.push((deltas.len(), median(&mut deltas)));
    }
    (
        best,
        found
            .into_iter()
            .filter(|(n, _)| *n == best)
            .map(|(_, d)| d)
            .collect(),
    )
}

impl Truth {
    pub fn from_media(media: &Media) -> Self {
        let mut truth = Self::default();
        for track in &media.tracks {
            let video = matches!(track.spec.config, CodecConfig::Avc { .. });
            let scale = f64::from(track.spec.timescale);
            let list = if video {
                &mut truth.video
            } else {
                &mut truth.audio
            };
            for s in &track.samples {
                let Some(pts) = s.pts else { continue };
                let key = if video {
                    video_key(&s.data)
                } else {
                    s.data.to_vec()
                };
                list.push((key, pts as f64 / scale));
            }
        }
        for list in [&mut truth.video, &mut truth.audio] {
            list.sort_by(|a, b| a.1.total_cmp(&b.1));
        }
        truth
    }

    pub fn source_start_gap(&self) -> f64 {
        self.audio.first().map_or(0.0, |s| s.1) - self.video.first().map_or(0.0, |s| s.1)
    }

    pub fn measure<'a>(&self, samples: impl Iterator<Item = (bool, &'a [u8], f64)>) -> GroundTruth {
        let (mut audio, mut video): (Vec<(Vec<u8>, f64)>, Vec<(Vec<u8>, f64)>) =
            (Vec::new(), Vec::new());
        for (is_video, data, pts) in samples {
            if is_video {
                video.push((video_key(data), pts));
            } else {
                audio.push((data.to_vec(), pts));
            }
        }
        for list in [&mut audio, &mut video] {
            list.sort_by(|a, b| a.1.total_cmp(&b.1));
        }
        let (audio_matched, audio_c) = candidates(&self.audio, &audio);
        let (video_matched, video_c) = candidates(&self.video, &video);
        assert!(
            audio_matched > 0 && video_matched > 0,
            "no emitted sample matched a source access unit (audio {audio_matched}, video {video_matched})"
        );
        let (audio_median, video_median) = audio_c
            .iter()
            .flat_map(|a| video_c.iter().map(move |v| (*a, *v)))
            .min_by(|x, y| (x.0 - x.1).abs().total_cmp(&(y.0 - y.1).abs()))
            .expect("a candidate pair");
        GroundTruth {
            audio_matched,
            video_matched,
            audio_median,
            video_median,
        }
    }
}
