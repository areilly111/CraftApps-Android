//! Image sequences and Broadcast WAV against ffmpeg: ffmpeg writes numbered stills (PNG, JPEG,
//! TIFF, BMP, WebP when available) and BWF files; the sequence frames must equal ffmpeg's decode of
//! the same image2 sequence (lossless formats exactly, JPEG within a few levels), and the BWF
//! TimeReference must give the start timecode.

mod common;

use std::path::Path;
use std::sync::Arc;

use common::*;
use filmcraft_media::sequence::{FrameLoader, ImageSequenceSource, Numbered, sequence_frames};
use filmcraft_media::{FrameRequest, MediaKind, MediaSource};
use filmcraft_time::FrameRate;

/// Twelve numbered stills written by ffmpeg, shared by every test that uses `ext` (#125).
///
/// ffmpeg writes into a private temporary directory, which is renamed into place only when all
/// twelve frames are there, so a test never reads a sequence another test is still writing (a
/// half-written PNG decodes as missing, and the sequence then holds the previous frame). A test that
/// loses the race keeps the winner's copy. The `.complete` marker tells a finished cache from a
/// directory left by an older, non-atomic generator (hence the `_v2` name).
fn make_sequence(ff: &Path, ext: &str, codec_args: &[&str]) -> Option<std::path::PathBuf> {
    let d = dir().join(format!("seq_{ext}_v2"));
    let complete = |d: &Path| d.join(".complete").exists();
    if complete(&d) {
        return Some(d);
    }
    let tmp = filmcraft_testkit::fixtures::temp_path(&d);
    std::fs::create_dir_all(&tmp).ok()?;
    let pattern = tmp.join(format!("img_%04d.{ext}"));
    let mut args = vec!["-y", "-v", "error", "-f", "lavfi", "-i", "testsrc2=size=96x64:rate=25", "-frames:v", "12"];
    args.extend_from_slice(codec_args);
    args.extend_from_slice(&["-f", "image2"]);
    let made = std::process::Command::new(ff).args(&args).arg(&pattern).status().is_ok_and(|st| st.success())
        && (1..=12).all(|n| tmp.join(format!("img_{n:04}.{ext}")).exists())
        && std::fs::write(tmp.join(".complete"), b"").is_ok();
    if made && std::fs::rename(&tmp, &d).is_ok() {
        return Some(d);
    }
    let _ = std::fs::remove_dir_all(&tmp);
    complete(&d).then_some(d)
}

fn loader() -> FrameLoader {
    Arc::new(|p: &str| std::fs::read(p))
}

fn open_sequence(d: &Path, ext: &str) -> ImageSequenceSource {
    let first = d.join(format!("img_0001.{ext}")).to_string_lossy().to_string();
    let n = Numbered::parse(&first).unwrap();
    let names: Vec<String> = std::fs::read_dir(d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    ImageSequenceSource::new("img_0001", sequence_frames(&n, &names), FrameRate::FPS_25, loader()).unwrap()
}

fn rgba(f: &filmcraft_frame::VideoFrame) -> Vec<u8> {
    match &f.data {
        filmcraft_frame::PixelData::Rgba8(p) => p.to_vec(),
        _ => panic!("still frames are RGBA"),
    }
}

fn check(ff: &Path, ext: &str, codec_args: &[&str], tol: u8) {
    let Some(d) = make_sequence(ff, ext, codec_args) else {
        eprintln!("SKIPPED: ffmpeg cannot write {ext} stills");
        return;
    };
    let src = open_sequence(&d, ext);
    assert_eq!(src.info().kind, MediaKind::ImageSequence);
    assert_eq!(src.frame_count(), 12);
    assert_eq!(src.info().duration, FrameRate::FPS_25.tick_of(12));
    let pattern = d.join(format!("img_%04d.{ext}"));
    let raw = ffmpeg_out(ff, &["-f", "image2", "-start_number", "1", "-i", pattern.to_str().unwrap(), "-f", "rawvideo", "-pix_fmt", "rgba", "-"]);
    let fsize = 96 * 64 * 4;
    assert_eq!(raw.len() / fsize, 12);
    let mut rng = Rng(5);
    let mut worst = 0u8;
    // in order, then random access
    let order: Vec<usize> = (0..12).chain((0..12).map(|_| rng.below(12) as usize)).collect();
    for i in order {
        let f = src.video_frame(FrameRequest::full(FrameRate::FPS_25.tick_of(i as i64))).unwrap();
        let ours = rgba(&f);
        let want = &raw[i * fsize..(i + 1) * fsize];
        let d = ours.iter().zip(want).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        worst = worst.max(d);
        assert!(d <= tol, "{ext}: frame {i} differs by {d}");
    }
    eprintln!("{ext} sequence: 12 frames, max diff {worst}");
}

#[test]
fn png_sequence_exact() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    check(&ff, "png", &[], 0);
}

#[test]
fn tiff_and_bmp_sequences_exact() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    check(&ff, "tif", &["-pix_fmt", "rgb24"], 0);
    check(&ff, "bmp", &[], 0);
}

#[test]
fn jpeg_sequence_close() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    // different IDCT and chroma upsampling than ffmpeg's: a few levels
    check(&ff, "jpg", &["-q:v", "2", "-pix_fmt", "yuvj444p"], 12);
}

#[test]
fn webp_sequence_lossless() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    check(&ff, "webp", &["-c:v", "libwebp", "-lossless", "1"], 0);
}

#[test]
fn missing_frames_hold_the_previous_frame() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let Some(d) = make_sequence(&ff, "png", &[]) else { return };
    // a private copy without frames 5 and 6
    let gap = filmcraft_testkit::fixtures::temp_path(&dir().join("seq_png_gap"));
    std::fs::create_dir_all(&gap).unwrap();
    for n in (1..=12).filter(|n| *n != 5 && *n != 6) {
        let name = format!("img_{n:04}.png");
        std::fs::copy(d.join(&name), gap.join(&name)).unwrap();
    }
    let _ = std::fs::remove_file(gap.join("img_0005.png"));
    let _ = std::fs::remove_file(gap.join("img_0006.png"));
    let src = open_sequence(&gap, "png");
    assert_eq!(src.missing_frames(), vec![4, 5]);
    let at = |i: i64| rgba(&src.video_frame(FrameRequest::full(FrameRate::FPS_25.tick_of(i))).unwrap());
    assert_eq!(at(4), at(3));
    assert_eq!(at(5), at(3));
    assert_ne!(at(6), at(3));
    let _ = std::fs::remove_dir_all(&gap);
}

#[test]
fn broadcast_wav_time_reference() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    // 01:00:00:00 at 23.976: 86 400 frames × 2002 samples
    let tr = (86_400u64 * 2002).to_string();
    let meta = format!("time_reference={tr}");
    let Some(f) = fixture(
        &ff,
        "bwf_1h.wav",
        &["-f", "lavfi", "-i", "sine=frequency=1000:sample_rate=48000", "-t", "0.5", "-c:a", "pcm_s24le", "-write_bext", "1", "-metadata", &meta],
    ) else {
        panic!("could not write BWF")
    };
    let src = filmcraft_codecs::open_bytes("bwf_1h.wav", bytes(&f)).unwrap();
    assert_eq!(src.info().container, "Broadcast WAV");
    assert_eq!(src.info().start_timecode, Some(86_400));
    // ffprobe agrees on the time reference
    if let Some(fp) = filmcraft_testkit::ffprobe() {
        let o = std::process::Command::new(fp).args(["-v", "error", "-show_entries", "format_tags=time_reference", "-of", "csv=p=0"]).arg(&f).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), tr);
    }
    // samples are exact
    let want = ffmpeg_audio_f32(&ff, &f, &[]);
    let got = src.audio(0, want.len(), 48_000).unwrap();
    assert_eq!(got.channels[0], want);
}
