//! Program streams (H.222.0 §2.5; ISO/IEC 11172-1 §2.4.3): packs, system headers, the program
//! stream map, PES packets, and the DVD sub-streams of private stream 1.

use std::collections::HashMap;

use crate::es::Splitter;
use crate::pes;
use crate::{ByteSource, Codec, Error, File, Format, LpcmFormat, Result, Stream, StreamId, Unit, Unwrap, read_some};

/// Sequential reader with a refillable window.
struct Win<'a, S: ByteSource + ?Sized> {
    src: &'a S,
    buf: Vec<u8>,
    start: u64,
}

impl<'a, S: ByteSource + ?Sized> Win<'a, S> {
    fn new(src: &'a S) -> Self {
        Self { src, buf: Vec::new(), start: 0 }
    }
    /// The bytes [pos, pos + n) (fewer at the end of the file).
    fn get(&mut self, pos: u64, n: usize) -> Result<&[u8]> {
        let have = pos >= self.start && pos + n as u64 <= self.start + self.buf.len() as u64;
        if !have {
            let want = n.max(1 << 20);
            self.buf.resize(want, 0);
            let got = read_some(self.src, pos, &mut self.buf)?;
            self.buf.truncate(got);
            self.start = pos;
        }
        let i = (pos - self.start) as usize;
        let end = (i + n).min(self.buf.len());
        Ok(&self.buf[i.min(end)..end])
    }
}

/// How many bytes of a private stream 1 payload precede the elementary stream, by sub-stream id
/// (DVD-Video: AC-3 / DTS 4, LPCM 7, sub-pictures 1).
fn sub_header(sub: u8) -> usize {
    match sub {
        0x80..=0x8F => 4,
        0xA0..=0xAF => 7,
        _ => 1,
    }
}

fn codec_for(stream_id: u8, sub: Option<u8>, psm: &HashMap<u8, u8>, mpeg1: bool) -> Codec {
    if let Some(&t) = psm.get(&stream_id) {
        let c = crate::ts::codec_of(t, &[], false);
        if !matches!(c, Codec::Unknown(_)) {
            return c;
        }
    }
    match (stream_id, sub) {
        (0xE0..=0xEF, _) => {
            if mpeg1 {
                Codec::Mpeg1Video
            } else {
                Codec::Mpeg2Video
            }
        }
        (0xC0..=0xDF, _) => Codec::MpegAudio,
        (0xBD, Some(0x80..=0x87)) => Codec::Ac3,
        (0xBD, Some(0x88..=0x8F)) => Codec::Dts,
        (0xBD, Some(0xA0..=0xAF)) => Codec::LpcmDvd,
        (0xBD, Some(0x20..=0x3F)) => Codec::Subtitle("DVD sub-pictures"),
        (id, _) => Codec::Unknown(id),
    }
}

/// DVD LPCM header bytes after the sub-stream id (and the frame count / pointer).
fn lpcm_format(h: &[u8]) -> Option<LpcmFormat> {
    let b = *h.get(4)?;
    let bits = [16, 20, 24, 0][(b >> 6) as usize];
    let sample_rate = [48_000, 96_000, 44_100, 32_000][((b >> 4) & 3) as usize];
    let channels = (b & 7) as u32 + 1;
    (bits > 0).then_some(LpcmFormat { sample_rate, channels, bits })
}

struct EsState {
    stream: Stream,
    split: Splitter,
}

/// Length of the packet (pack header, system header, PES…) starting at `b` (which begins with a
/// start code), `None` if it is not one.
fn packet_len(b: &[u8]) -> Option<usize> {
    if b.len() < 6 || b[0] != 0 || b[1] != 0 || b[2] != 1 {
        return None;
    }
    match b[3] {
        0xBA => {
            if b[4] >> 6 == 1 {
                let stuff = (*b.get(13)? & 7) as usize;
                Some(14 + stuff)
            } else if b[4] >> 4 == 2 {
                Some(12)
            } else {
                None
            }
        }
        0xB9 => Some(4),
        0xBB..=0xFF => Some(6 + u16::from_be_bytes([b[4], b[5]]) as usize),
        _ => None,
    }
}

pub(crate) fn scan<S: ByteSource + ?Sized>(src: &S, mpeg1: bool) -> Result<File> {
    let len = src.len();
    let mut win = Win::new(src);
    let mut pos = 0u64;
    let mut streams: HashMap<(u8, Option<u8>), EsState> = HashMap::new();
    let mut order = Vec::new();
    let mut psm: HashMap<u8, u8> = HashMap::new();
    let mut unwrap = Unwrap::default();
    let mut scr: Option<(i64, i64)> = None;
    let mut resyncs = 0u64;
    while pos + 4 <= len {
        let head = win.get(pos, 32)?.to_vec();
        let Some(plen) = packet_len(&head) else {
            // resynchronise on the next pack header or PES start code
            resyncs += 1;
            let mut p = pos + 1;
            let found = loop {
                let w = win.get(p, 1 << 16)?.to_vec();
                if w.len() < 4 {
                    break None;
                }
                if let Some(i) = w.windows(4).position(|x| x[0] == 0 && x[1] == 0 && x[2] == 1 && (x[3] == 0xBA || (0xBD..=0xEF).contains(&x[3]))) {
                    break Some(p + i as u64);
                }
                p += (w.len() - 3) as u64;
            };
            match found {
                Some(f) => {
                    pos = f;
                    continue;
                }
                None => break,
            }
        };
        let id = head[3];
        if id == 0xBA {
            // SCR (MPEG-2: 33 bits base, 9 bits extension; MPEG-1: 33 bits)
            let v = if head[4] >> 6 == 1 {
                let b = &head[4..10];
                let base = (((b[0] >> 3) & 7) as i64) << 30
                    | ((b[0] & 3) as i64) << 28
                    | (b[1] as i64) << 20
                    | ((b[2] >> 3) as i64) << 15
                    | ((b[2] & 3) as i64) << 13
                    | (b[3] as i64) << 5
                    | (b[4] >> 3) as i64;
                let ext = (((b[4] & 3) as i64) << 7) | (b[5] >> 1) as i64;
                base * 300 + ext
            } else {
                pes::timestamp(&head[4..9]) * 300
            };
            scr = Some(scr.map_or((v, v), |(a, b): (i64, i64)| (a.min(v), b.max(v))));
        } else if id == 0xBC {
            // program stream map: elementary stream id → stream_type
            let body = win.get(pos, plen)?.to_vec();
            if body.len() >= 12 {
                let info = u16::from_be_bytes([body[8], body[9]]) as usize;
                let mut i = 10 + info;
                if i + 2 <= body.len() {
                    let map_len = u16::from_be_bytes([body[i], body[i + 1]]) as usize;
                    i += 2;
                    let end = (i + map_len).min(body.len());
                    while i + 4 <= end {
                        psm.insert(body[i + 1], body[i]);
                        i += 4 + u16::from_be_bytes([body[i + 2], body[i + 3]]) as usize;
                    }
                }
            }
        } else if id == 0xBD || (0xC0..=0xEF).contains(&id) || id == 0xFD {
            let pkt = win.get(pos, plen)?.to_vec();
            if pkt.len() < plen {
                break; // truncated
            }
            if let Some(h) = pes::parse(&pkt) {
                let payload = &pkt[h.header_len.min(plen)..];
                let (sub, skip) = if id == 0xBD {
                    match payload.first() {
                        Some(&s) => (Some(s), sub_header(s).min(payload.len())),
                        None => (None, 0),
                    }
                } else {
                    (None, 0)
                };
                let key = (id, sub);
                let e = streams.entry(key).or_insert_with(|| {
                    let codec = codec_for(id, sub, &psm, mpeg1);
                    let lpcm = if codec == Codec::LpcmDvd { lpcm_format(&payload[1.min(payload.len())..]) } else { None };
                    order.push(key);
                    let stream = Stream {
                        id: StreamId::Ps { stream_id: id, sub_id: sub },
                        stream_type: psm.get(&id).copied().unwrap_or(0),
                        codec: codec.clone(),
                        language: None,
                        units: Vec::new(),
                        lpcm,
                        pes_packets: 0,
                        skipped_bytes: 0,
                    };
                    EsState { stream, split: Splitter::new(&codec) }
                });
                e.stream.pes_packets += 1;
                e.split.pes_start(h.pts.map(|t| unwrap.unwrap(t)), h.dts.map(|t| unwrap.unwrap(t)));
                e.split.feed(pos, &payload[skip..]);
            }
        }
        pos += plen as u64;
    }
    let mut warnings = Vec::new();
    if resyncs > 0 {
        warnings.push(format!("resynchronised {resyncs} time(s)"));
    }
    let mut out = Vec::new();
    for key in order {
        let Some(mut e) = streams.remove(&key) else { continue };
        e.split.finish();
        e.stream.units = std::mem::take(&mut e.split.units);
        e.stream.skipped_bytes = e.split.skipped;
        out.push(e.stream);
    }
    if out.is_empty() {
        return Err(Error::Invalid("program stream without elementary streams".into()));
    }
    Ok(File { format: Format::Ps { mpeg1 }, streams: out, program: None, pcr_range: scr, warnings })
}

/// Collect unit `u` of stream `st`, starting at the PES packet at `u.pos`.
pub(crate) fn read<S: ByteSource + ?Sized>(src: &S, st: &Stream, u: &Unit, out: &mut Vec<u8>) -> Result<()> {
    let StreamId::Ps { stream_id, sub_id } = st.id else { return Err(Error::Invalid("not a PS stream".into())) };
    let need = u.size as usize;
    let mut skip = u.offset as usize;
    let mut pos = u.pos;
    let len = src.len();
    let mut win = Win::new(src);
    while out.len() < need && pos + 4 <= len {
        let head = win.get(pos, 32)?.to_vec();
        let Some(plen) = packet_len(&head) else {
            // corrupt: stop at what we have
            return Ok(());
        };
        if head[3] == stream_id {
            let pkt = win.get(pos, plen)?.to_vec();
            if let Some(h) = pes::parse(&pkt) {
                let payload = &pkt[h.header_len.min(pkt.len())..];
                let matches = stream_id != 0xBD || payload.first().copied() == sub_id;
                if matches {
                    let s0 = if stream_id == 0xBD { sub_id.map_or(0, sub_header).min(payload.len()) } else { 0 };
                    let mut bytes = &payload[s0..];
                    if skip > 0 {
                        let s = skip.min(bytes.len());
                        skip -= s;
                        bytes = &bytes[s..];
                    }
                    let take = bytes.len().min(need - out.len());
                    out.extend_from_slice(&bytes[..take]);
                }
            }
        }
        pos += plen as u64;
    }
    Ok(())
}
