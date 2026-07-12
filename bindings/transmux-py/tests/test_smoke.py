"""Smoke test for transmux-py against a real committed MPEG-2 TS capture.

Validates the binding against `fixtures/ts/h264_aac.ts` — the deterministic,
real 2-track (H.264 video + AAC audio) capture the `transmux` Rust crate's own
test suite (`transmux/tests/ts_demux.rs`, `smooth.rs`, `golden_gate.rs`, etc.)
uses as its oracle-backed fixture. Its exact sample counts (75 video / 131
audio) and codec identity are cross-checked there against ffprobe/ffmpeg
oracles, so this test's expected numbers are not invented — they mirror the
Rust-side oracle values. Run with `pytest` after `maturin develop`.
"""

import os

import transmux_py as tm

# Repo-relative path to the committed real capture (shared across the
# workspace's own transmux test suite, not fixture data private to this
# binding).
FIXTURE = os.path.join(
    os.path.dirname(__file__), "..", "..", "..", "fixtures", "ts", "h264_aac.ts"
)


def _demux_fixture():
    with open(FIXTURE, "rb") as f:
        data = f.read()
    return tm.demux_ts(data)


def test_demuxes_two_tracks():
    media = _demux_fixture()
    assert isinstance(media, dict)
    assert media["movie_timescale"] > 0
    assert len(media["tracks"]) == 2, "fixture is a 2-track (H.264 + AAC) capture"


def test_video_track_is_avc_with_sync_samples():
    media = _demux_fixture()
    video = next(t for t in media["tracks"] if t["codec"] == "avc")
    assert video["timescale"] == 90000, "TS video PES clock is 90 kHz"
    assert video["width"] == 320
    assert video["height"] == 240
    # RFC 6381 avc1.PPCCLL string, derived from the recovered avcC fields.
    assert video["codec_string"] is not None
    assert video["codec_string"].startswith("avc1.")
    assert len(video["samples"]) == 75, "oracle count from transmux/tests/smooth.rs"
    assert any(s["is_sync"] for s in video["samples"]), "expected at least one keyframe"
    first = video["samples"][0]
    assert isinstance(first["data"], bytes)
    assert len(first["data"]) > 0
    assert first["is_sync"], "the first sample of this fixture is a keyframe"
    assert first["duration"] > 0


def test_audio_track_is_aac():
    media = _demux_fixture()
    audio = next(t for t in media["tracks"] if t["codec"] == "aac")
    assert audio["timescale"] == 44100
    assert audio["channel_count"] == 1
    assert audio["sample_rate"] == 44100
    assert audio["codec_string"] == "mp4a.40.2"  # AAC-LC (AudioObjectType 2)
    assert len(audio["samples"]) == 131, "oracle count from transmux/tests/smooth.rs"
    first = audio["samples"][0]
    assert isinstance(first["data"], bytes)
    assert len(first["data"]) > 0
    # Every AAC access unit is a sync sample (no inter-frame prediction).
    assert all(s["is_sync"] for s in audio["samples"])


def test_source_pids_are_distinct():
    media = _demux_fixture()
    pids = {t["source_pid"] for t in media["tracks"]}
    assert None not in pids, "TS-demuxed tracks must carry a source PID"
    assert len(pids) == 2, "video and audio must be on distinct PIDs"
