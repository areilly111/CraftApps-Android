# filmcraft-matroska

A clean-room Matroska (MKV) and WebM demuxer, plus a minimal muxer, written from the public
specifications: RFC 8794 (EBML), RFC 9559 (Matroska), the Matroska codec mappings and the WebM
container guidelines. Layer L0: it depends only on `std`, has no `unsafe`, and builds for
`wasm32-unknown-unknown`. No GPL/LGPL code (FFmpeg, libmatroska, mkvtoolnix) was consulted;
FFmpeg is used only as an external test oracle.

## Demuxing

```rust
use filmcraft_matroska::{Demuxer, TrackKind, open};

// Sample tables, same shape as filmcraft-isobmff (any ByteSource: &[u8], Vec<u8>, Arc<[u8]>, File…)
let file = std::fs::File::open("clip.mkv")?;
let mkv = open(&file)?;
let v = mkv.track_of_kind(TrackKind::Video).unwrap();
let k = mkv.keyframe_before(v, 2_000_000_000).unwrap();   // sample index
let frame = mkv.read_sample(&file, v, k)?;

// Streaming packets + seeking (from a ByteSource, any Read + Seek, or a byte slice)
let mut d = Demuxer::from_reader(std::io::BufReader::new(std::fs::File::open("clip.webm")?))?;
let sp = d.seek(v, 2_000_000_000)?;          // lands on the keyframe with pts ≤ 2 s
while let Some(p) = d.next_packet()? {        // file order, all tracks
    // p.track, p.pts (ticks), p.pts_ns, p.duration, p.keyframe, p.data
}
```

- **Open** (`open`, `open_with(src, &OpenOptions { index, verify_crc })`): EBML header (DocType
  `matroska`/`webm`), the first Segment, and its level-1 elements: SeekHead (followed for elements
  after the clusters), Info, Tracks, Cues, Chapters, Tags, Attachments (listed; payload located by
  offset/size, never read). With `index: true` (default) every Cluster is scanned — element and block
  headers only, never frame payloads — to build per-track `Sample` tables (`offset`, `size`, `pts`,
  `duration`, `keyframe`, cluster, block offset, lace index) and keyframe lists. With `index: false`
  open stops at the first Cluster.
- **Timestamps.** Matroska ticks are `TimestampScale` ns (usually 1 ms); `Track::timebase` is that
  as a reduced fraction of a second. `pts` is in ticks, `pts_ns` in exact nanoseconds. `CodecDelay`
  is subtracted (RFC 9559), as FFmpeg does (Opus/Vorbis priming gives negative pts). Laced frames get
  `block ts + i × (BlockDuration or DefaultDuration)/n`; if neither is known, later laces repeat the
  block timestamp and have `duration == 0` (`Packet::lace` tells which).
- **Keyframes:** SimpleBlock keyframe flag; BlockGroup = keyframe iff it has no ReferenceBlock.
- **Lacing:** Xiph, fixed-size, EBML. **Unknown sizes:** Segment (live/streamed output) and
  Clusters end at the next top-level element. **CRC-32:** optional verification of level-1
  elements and clusters; mismatches go to `MkvFile::crc_errors` (not fatal). **Void** skipped.
- **Damage:** undecodable/oversized elements trigger a byte scan for the next plausible level-1
  element (Cluster whose first child is Timestamp, or a sized metadata element); a truncated final
  block is dropped. Problems are listed in `MkvFile::warnings`.
- **Seeking** (`Demuxer::seek(track, time_ns)`): uses the sample index when present; otherwise Cues
  (refined by scanning the cued cluster for the exact keyframe); otherwise builds the index by
  scanning all clusters. `Demuxer::keyframe_index(track)` returns the keyframe list.
- **Tracks:** number/UID/type/flags, `DefaultDuration`, name, language (+BCP 47), CodecID,
  CodecPrivate, CodecName, `CodecDelay`, `SeekPreRoll`; `VideoInfo` (pixel/display size, crop,
  interlacing, stereo, alpha mode, `pixel_aspect()`), `Colour` (matrix, range, transfer, primaries,
  subsampling/siting, MaxCLL/MaxFALL, SMPTE 2086 mastering metadata); `AudioInfo` (rate, output
  rate, channels, bit depth); `ContentEncoding`s.
- **Content encodings:** header stripping is undone in every packet / `read_sample`. zlib/bzlib/
  lzo compression and encryption are reported (`Track::frames_readable() == false`); packets are
  then returned as stored and `read_sample` fails with `Unsupported`.

### Codec mapping (`Codec`)

| CodecID | `Codec` | payload |
|---|---|---|
| `V_MPEG4/ISO/AVC` | `Avc` | avcC (CodecPrivate) |
| `V_MPEGH/ISO/HEVC` | `Hevc` | hvcC |
| `V_VP8`, `V_VP9` | `Vp8`, `Vp9` | VP9 CodecPrivate features |
| `V_AV1` | `Av1` | av1C |
| `V_PRORES` | `ProRes` | FourCC; frames get the 8-byte `size`+`icpf` header re-added |
| `V_MJPEG`, `V_MS/VFW/FOURCC` (MJPG) | `Mjpeg` | |
| `V_MS/VFW/FOURCC` (other) | `VfwFourcc` | BITMAPINFOHEADER |
| `A_AAC`, legacy `A_AAC/MPEG{2,4}/…` | `Aac` | AudioSpecificConfig (synthesised for legacy IDs, incl. SBR) |
| `A_OPUS` | `Opus` | OpusHead |
| `A_VORBIS` | `Vorbis` | three headers split from Xiph-laced CodecPrivate |
| `A_FLAC` | `Flac` | `fLaC` + metadata blocks |
| `A_PCM/INT/LIT`, `A_PCM/INT/BIG`, `A_PCM/FLOAT/IEEE` | `Pcm` | bit depth from Audio |
| `A_AC3`, `A_EAC3`, `A_MPEG/L3`, `A_MPEG/L2` | `Ac3`, `Eac3`, `Mp3`, `Mp2` | |
| `S_TEXT/UTF8` | `SubRip` | |
| `S_TEXT/WEBVTT`, `D_WEBVTT/*` (WebM) | `WebVtt` | WebM blocks keep `id\nsettings\npayload` |
| `S_TEXT/ASS`, `S_TEXT/SSA` | `Ass` | script header |
| anything else | `Other(id)` | `Track::codec_private` |

`Codec::name()` matches FFmpeg's `codec_name` for these.

## Muxing (minimal)

`MkvWriter::new(w: Write + Seek, Vec<TrackSpec>, MuxOptions)` → `write_frame(track, pts_ns,
keyframe, data, duration_ns)` → `finish()`. Writes the EBML header, Segment, SeekHead, Info (with
Duration), Tracks, Clusters of SimpleBlocks (new cluster at a video keyframe after
`cluster_duration_ns`), BlockGroups for explicit durations, optional audio lacing (Xiph, EBML,
fixed-size), and Cues (video keyframes, or the first frame of each track per cluster). No
CRC-32, Chapters, Tags, Attachments, or content encodings.

## Tests

- Unit: EBML VINTs (known/unknown sizes, IDs, signed lace deltas), typed values, CRC-32, all lacing
  modes, codec mapping, a hand-built file (unknown-size Segment/Clusters, header stripping,
  BlockGroups, Colour + mastering metadata, Chapters, Tags, Attachments, CRC-32 pass/fail, Cue-less
  seek), and mux → demux round trips for every lacing mode.
- Oracle (`tests/oracle.rs`, skipped without ffmpeg/ffprobe; fixtures in `target/fixtures/matroska`):
  H.264+AAC, VP9+Opus WebM, FLAC, Vorbis, SRT+ASS subtitles, WebVTT WebM, Cues at front, a
  streamed (pipe) file without Cues, HEVC with HDR colour, ProRes+PCM, MJPEG+AC-3, AV1. Every packet
  is compared with `ffprobe -show_packets` (stream, size, keyframe flag, pts, duration), plus codec
  names, time bases, dimensions, audio params and duration; the sample index must equal the packet
  stream. Seeks (indexed, Cues-only, and index-on-demand) must land on the latest keyframe ≤ target.
  Our muxer's laced output (Xiph/EBML/fixed) is read back by ffprobe and compared the same way.
  A damaged file must resync and deliver the packets after the damage; truncated files must not fail.

## Limitations

- Only the first Segment is read (no linked/chained segments, no ordered-chapter editions playback).
- zlib/bzlib/lzo compressed and encrypted tracks are detected but their frames are not decoded.
- BlockAdditions (e.g. VP9 alpha, WebVTT settings in Matroska) are skipped; `EncryptedBlock` ignored.
- `TrackTimestampScale` (deprecated) is ignored.
- Lace timestamps without any duration information repeat the block timestamp (FFmpeg derives them
  from codec parsers).
