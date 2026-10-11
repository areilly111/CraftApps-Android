//! Transport streams (H.222.0 §2.4.3): packets, PSI (PAT/PMT), PES reassembly.

use std::collections::HashMap;

use crate::es::Splitter;
use crate::pes::{self, PesHeader};
use crate::{ByteSource, Codec, Error, File, Format, Result, Stream, StreamId, Unit, Unwrap, read_some};

/// CRC-32 of PSI sections (MSB-first, polynomial 0x04C11DB7, initial value all ones; Annex A).
pub(crate) fn crc32_mpeg(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= (b as u32) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04C1_1DB7 } else { crc << 1 };
        }
    }
    crc
}

/// A packet's fields.
struct Packet<'a> {
    pusi: bool,
    pid: u16,
    cc: u8,
    has_payload: bool,
    discontinuity: bool,
    pcr: Option<i64>,
    payload: &'a [u8],
}

fn packet(p: &[u8]) -> Option<Packet<'_>> {
    if p.len() < 188 || p[0] != 0x47 {
        return None;
    }
    let pusi = p[1] & 0x40 != 0;
    let pid = (((p[1] & 0x1F) as u16) << 8) | p[2] as u16;
    let afc = (p[3] >> 4) & 3;
    let cc = p[3] & 15;
    let mut start = 4;
    let mut pcr = None;
    let mut discontinuity = false;
    if afc & 2 != 0 {
        let al = p[4] as usize;
        start = 5 + al;
        if start > 188 {
            return None;
        }
        if al > 0 {
            let flags = p[5];
            discontinuity = flags & 0x80 != 0;
            if flags & 0x10 != 0 && al >= 7 {
                let base = ((p[6] as i64) << 25) | ((p[7] as i64) << 17) | ((p[8] as i64) << 9) | ((p[9] as i64) << 1) | (p[10] as i64 >> 7);
                let ext = (((p[10] & 1) as i64) << 8) | p[11] as i64;
                pcr = Some(base * 300 + ext);
            }
        }
    }
    let has_payload = afc & 1 != 0;
    Some(Packet { pusi, pid, cc, has_payload, discontinuity, pcr, payload: if has_payload { &p[start..188] } else { &[] } })
}

/// Reassembles a PES header that may span packets; yields each packet's elementary-stream bytes.
#[derive(Default)]
pub(crate) struct PesAsm {
    hdr: Vec<u8>,
    waiting: bool,
}

impl PesAsm {
    /// The PES header completed by this packet (if any) and the packet's ES payload bytes.
    pub fn packet<'a>(&mut self, pusi: bool, payload: &'a [u8]) -> (Option<PesHeader>, &'a [u8]) {
        if pusi {
            self.hdr.clear();
            self.waiting = true;
        } else if !self.waiting {
            return (None, payload);
        }
        let before = self.hdr.len();
        self.hdr.extend_from_slice(payload);
        if let Some(h) = pes::parse(&self.hdr)
            && self.hdr.len() >= h.header_len
        {
            self.waiting = false;
            let skip = (h.header_len - before).min(payload.len());
            return (Some(h), &payload[skip..]);
        }
        if self.hdr.len() > 1024 || (self.hdr.len() >= 6 && !(self.hdr[0] == 0 && self.hdr[1] == 0 && self.hdr[2] == 1)) {
            // not a PES packet after all: drop until the next unit start
            self.waiting = false;
            self.hdr.clear();
            return (None, &[]);
        }
        (None, &[])
    }
}

/// PSI section reassembly for one PID.
#[derive(Default)]
struct Psi {
    buf: Vec<u8>,
}

impl Psi {
    /// Feed a packet payload; returns completed sections.
    fn feed(&mut self, pusi: bool, payload: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        if pusi {
            let Some(&ptr) = payload.first() else { return out };
            let ptr = ptr as usize;
            if !self.buf.is_empty() {
                self.buf.extend_from_slice(payload.get(1..1 + ptr).unwrap_or(&[]));
                self.drain(&mut out);
            }
            self.buf = payload.get(1 + ptr..).unwrap_or(&[]).to_vec();
        } else if !self.buf.is_empty() {
            self.buf.extend_from_slice(payload);
        }
        self.drain(&mut out);
        out
    }

    fn drain(&mut self, out: &mut Vec<Vec<u8>>) {
        while self.buf.len() >= 3 {
            if self.buf[0] == 0xFF {
                self.buf.clear();
                return;
            }
            let total = 3 + ((((self.buf[1] & 0x0F) as usize) << 8) | self.buf[2] as usize);
            if self.buf.len() < total {
                if total > 4096 {
                    self.buf.clear();
                }
                return;
            }
            out.push(self.buf[..total].to_vec());
            self.buf.drain(..total);
        }
    }
}

/// Elementary stream of the selected program.
struct EsState {
    stream: Stream,
    split: Splitter,
    asm: PesAsm,
    cc: Option<u8>,
}

/// Map a PMT stream_type (and the ES descriptors) to a codec. `hdmv`: a Blu-ray / AVCHD stream
/// (registration descriptor "HDMV", or 192-byte packets).
pub(crate) fn codec_of(stream_type: u8, desc: &[u8], hdmv: bool) -> Codec {
    let mut tags = Vec::new();
    let mut reg = None;
    let mut i = 0;
    while i + 2 <= desc.len() {
        let (tag, len) = (desc[i], desc[i + 1] as usize);
        let body = desc.get(i + 2..i + 2 + len).unwrap_or(&[]);
        tags.push(tag);
        if tag == 5 && body.len() >= 4 {
            reg = Some([body[0], body[1], body[2], body[3]]);
        }
        i += 2 + len;
    }
    match stream_type {
        0x01 => Codec::Mpeg1Video,
        0x02 => Codec::Mpeg2Video,
        0x03 | 0x04 => Codec::MpegAudio,
        0x0F => Codec::AacAdts,
        0x11 => Codec::AacLatm,
        0x1B => Codec::H264,
        0x24 => Codec::Hevc,
        0x80 if hdmv => Codec::LpcmBluray,
        0x80 => Codec::Mpeg2Video,
        0x81 => Codec::Ac3,
        0x82 | 0x85 | 0x86 | 0xA2 if hdmv => Codec::Dts,
        0x83 if hdmv => Codec::TrueHd,
        0x84 | 0xA1 if hdmv => Codec::Eac3,
        0x87 => Codec::Eac3,
        0x90 if hdmv => Codec::Subtitle("PGS subtitles"),
        0x92 if hdmv => Codec::Subtitle("Text subtitles"),
        0x06 => {
            if tags.contains(&0x6A) || reg == Some(*b"AC-3") {
                Codec::Ac3
            } else if tags.contains(&0x7A) || reg == Some(*b"EAC3") {
                Codec::Eac3
            } else if tags.contains(&0x7B) || matches!(reg, Some([b'D', b'T', b'S', _])) {
                Codec::Dts
            } else if tags.contains(&0x59) {
                Codec::Subtitle("DVB subtitles")
            } else if tags.contains(&0x56) {
                Codec::Subtitle("Teletext")
            } else {
                Codec::Unknown(6)
            }
        }
        t => Codec::Unknown(t),
    }
}

fn language(desc: &[u8]) -> Option<String> {
    let mut i = 0;
    while i + 2 <= desc.len() {
        let (tag, len) = (desc[i], desc[i + 1] as usize);
        if tag == 0x0A && len >= 3 {
            let b = desc.get(i + 2..i + 5)?;
            return Some(String::from_utf8_lossy(b).trim().to_string()).filter(|s| !s.is_empty());
        }
        i += 2 + len;
    }
    None
}

/// First file offset at which `packet_size`-byte packets line up (sync byte at `sync_offset`).
fn first_sync(head: &[u8], packet_size: usize, sync_offset: usize) -> Option<usize> {
    (0..packet_size.min(head.len())).find(|&s| (0..5).all(|k| head.get(s + sync_offset + k * packet_size) == Some(&0x47)))
}

pub(crate) fn scan<S: ByteSource + ?Sized>(src: &S, packet_size: usize, sync_offset: usize) -> Result<File> {
    let len = src.len();
    let mut head = vec![0u8; packet_size * 8];
    let n = read_some(src, 0, &mut head)?;
    let mut off = first_sync(&head[..n], packet_size, sync_offset).ok_or(Error::NotMpeg)? as u64;
    let mut warnings: Vec<String> = Vec::new();
    let mut pat = Psi::default();
    let mut pmt_pid: Option<u16> = None;
    let mut program = None;
    let mut pmt = Psi::default();
    let mut es: HashMap<u16, EsState> = HashMap::new();
    let mut order: Vec<u16> = Vec::new();
    let mut pmt_version: Option<u8> = None;
    let mut unwrap = Unwrap::default();
    let mut pcr_unwrap = 0i64;
    let mut last_pcr: Option<i64> = None;
    let mut pcr_range: Option<(i64, i64)> = None;
    let (mut lost_sync, mut cc_errors, mut crc_errors) = (0u64, 0u64, 0u64);
    const BLOCK: usize = 4096;
    let mut block = vec![0u8; BLOCK * packet_size];
    while off + packet_size as u64 <= len {
        let got = read_some(src, off, &mut block)?;
        let whole = got / packet_size;
        if whole == 0 {
            break;
        }
        let mut k = 0;
        while k < whole {
            let pos = off + (k * packet_size) as u64;
            let raw = &block[k * packet_size..(k + 1) * packet_size];
            let Some(pk) = packet(&raw[sync_offset..]) else {
                // lost sync: find the next offset where packets line up again
                lost_sync += 1;
                let rest = &block[k * packet_size + 1..got];
                match first_sync(rest, packet_size, sync_offset).or_else(|| rest.iter().position(|&b| b == 0x47).map(|p| p.saturating_sub(sync_offset))) {
                    Some(s) => {
                        off = pos + 1 + s as u64;
                    }
                    None => off = pos + got as u64,
                }
                k = usize::MAX;
                break;
            };
            k += 1;
            if let Some(pcr) = pk.pcr {
                // 33-bit base × 300 wraps at 2^33·300
                let wrap = (1i64 << 33) * 300;
                let mut v = pcr + pcr_unwrap;
                if let Some(l) = last_pcr
                    && v < l - wrap / 2
                {
                    pcr_unwrap += wrap;
                    v += wrap;
                }
                last_pcr = Some(v);
                pcr_range = Some(pcr_range.map_or((v, v), |(a, b)| (a.min(v), b.max(v))));
            }
            if pk.pid == 0 {
                for sec in pat.feed(pk.pusi, pk.payload) {
                    if sec.len() < 12 || sec[0] != 0 {
                        continue;
                    }
                    if crc32_mpeg(&sec) != 0 {
                        crc_errors += 1;
                        continue;
                    }
                    let body = &sec[8..sec.len() - 4];
                    for e in body.as_chunks::<4>().0 {
                        let num = u16::from_be_bytes([e[0], e[1]]);
                        let pid = (((e[2] & 0x1F) as u16) << 8) | e[3] as u16;
                        if num != 0 && pmt_pid.is_none() {
                            pmt_pid = Some(pid);
                            program = Some(num);
                        }
                    }
                }
                continue;
            }
            if Some(pk.pid) == pmt_pid {
                for sec in pmt.feed(pk.pusi, pk.payload) {
                    if sec.len() < 16 || sec[0] != 2 {
                        continue;
                    }
                    if crc32_mpeg(&sec) != 0 {
                        crc_errors += 1;
                        continue;
                    }
                    let version = (sec[5] >> 1) & 0x1F;
                    if pmt_version == Some(version) {
                        continue;
                    }
                    pmt_version = Some(version);
                    let pinfo = (((sec[10] & 0x0F) as usize) << 8) | sec[11] as usize;
                    let prog_desc = sec.get(12..12 + pinfo).unwrap_or(&[]);
                    let hdmv = packet_size == 192 || prog_desc.windows(4).any(|w| w == b"HDMV");
                    let mut i = 12 + pinfo;
                    let end = sec.len() - 4;
                    while i + 5 <= end {
                        let st = sec[i];
                        let pid = (((sec[i + 1] & 0x1F) as u16) << 8) | sec[i + 2] as u16;
                        let ilen = (((sec[i + 3] & 0x0F) as usize) << 8) | sec[i + 4] as usize;
                        let desc = sec.get(i + 5..(i + 5 + ilen).min(end)).unwrap_or(&[]);
                        i += 5 + ilen;
                        if es.contains_key(&pid) {
                            continue;
                        }
                        let mut codec = codec_of(st, desc, hdmv);
                        if codec == Codec::Unknown(6) && prog_desc.windows(4).any(|w| w == b"AC-3") {
                            codec = Codec::Ac3;
                        }
                        let stream = Stream {
                            id: StreamId::Pid(pid),
                            stream_type: st,
                            codec: codec.clone(),
                            language: language(desc),
                            units: Vec::new(),
                            lpcm: None,
                            pes_packets: 0,
                            skipped_bytes: 0,
                        };
                        es.insert(pid, EsState { stream, split: Splitter::new(&codec), asm: PesAsm::default(), cc: None });
                        order.push(pid);
                    }
                }
                continue;
            }
            let Some(e) = es.get_mut(&pk.pid) else { continue };
            if pk.has_payload {
                if let Some(prev) = e.cc
                    && !pk.discontinuity
                    && pk.cc != (prev + 1) & 15
                    && pk.cc != prev
                {
                    cc_errors += 1;
                }
                e.cc = Some(pk.cc);
            }
            let (hdr, bytes) = e.asm.packet(pk.pusi, pk.payload);
            if let Some(h) = hdr {
                e.stream.pes_packets += 1;
                let pts = h.pts.map(|t| unwrap.unwrap(t));
                let dts = h.dts.map(|t| unwrap.unwrap(t));
                e.split.pes_start(pts, dts);
            }
            e.split.feed(pos, bytes);
        }
        if k != usize::MAX {
            off += (whole * packet_size) as u64;
        }
    }
    if lost_sync > 0 {
        warnings.push(format!("lost packet sync {lost_sync} time(s)"));
    }
    if cc_errors > 0 {
        warnings.push(format!("{cc_errors} continuity counter error(s)"));
    }
    if crc_errors > 0 {
        warnings.push(format!("{crc_errors} PSI section(s) with bad CRC"));
    }
    if pmt_pid.is_none() {
        warnings.push("no program association table".into());
    }
    let mut streams = Vec::new();
    for pid in order {
        let Some(mut e) = es.remove(&pid) else { continue };
        e.split.finish();
        e.stream.units = std::mem::take(&mut e.split.units);
        e.stream.skipped_bytes = e.split.skipped;
        streams.push(e.stream);
    }
    Ok(File { format: Format::Ts { packet_size, sync_offset }, streams, program, pcr_range, warnings })
}

/// Collect unit `u` of stream `st`: from the packet at `u.pos`, the PID's elementary-stream bytes.
pub(crate) fn read<S: ByteSource + ?Sized>(src: &S, packet_size: usize, sync_offset: usize, st: &Stream, u: &Unit, out: &mut Vec<u8>) -> Result<()> {
    let StreamId::Pid(pid) = st.id else { return Err(Error::Invalid("not a TS stream".into())) };
    let need = u.size as usize;
    let mut skip = u.offset as usize;
    let mut off = u.pos;
    let mut asm = PesAsm::default();
    let mut first = true;
    let mut block = vec![0u8; 512 * packet_size];
    while out.len() < need {
        let got = read_some(src, off, &mut block)?;
        let whole = got / packet_size;
        if whole == 0 {
            break;
        }
        for k in 0..whole {
            let raw = &block[k * packet_size..(k + 1) * packet_size];
            let Some(pk) = packet(&raw[sync_offset..]) else { return Ok(()) };
            if pk.pid != pid {
                continue;
            }
            // a unit starting in a PES packet's first packet: the header is skipped
            let (_, bytes) = if first && !pk.pusi { (None, pk.payload) } else { asm.packet(pk.pusi, pk.payload) };
            first = false;
            let bytes = if skip > 0 {
                let s = skip.min(bytes.len());
                skip -= s;
                &bytes[s..]
            } else {
                bytes
            };
            let take = bytes.len().min(need - out.len());
            out.extend_from_slice(&bytes[..take]);
            if out.len() >= need {
                return Ok(());
            }
        }
        off += (whole * packet_size) as u64;
    }
    Ok(())
}
