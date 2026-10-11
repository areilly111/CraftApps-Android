//! A store-only (uncompressed) ZIP archive: several exported files as one download. The files are
//! already compressed (PNG, JPEG, PDF…), so storing them keeps the archive fast to write and read.

use flate2::Crc;

/// The DOS date of every entry: 1980-01-01, the earliest a ZIP date can say (no clock on the web,
/// and a fixed date keeps archives of the same files identical).
const DOS_DATE: u16 = (1 << 5) | 1;
/// General purpose flag: file names are UTF-8.
const UTF8_NAMES: u16 = 1 << 11;
/// Version needed to extract: 2.0.
const VERSION: u16 = 20;

/// The archive of `files` (`(name, bytes)`; `/` separates folders in a name), stored without
/// compression. Fails past the limits of a ZIP without its 64-bit extension (65535 files, 4 GB).
pub fn store<N: AsRef<str>, B: AsRef<[u8]>>(files: &[(N, B)]) -> Result<Vec<u8>, String> {
    let too_big = || "too much to zip: at most 65535 files and 4 GB".to_string();
    let count = u16::try_from(files.len()).map_err(|_| too_big())?;
    let mut out = Vec::with_capacity(files.iter().map(|(n, b)| 76 + 2 * n.as_ref().len() + b.as_ref().len()).sum::<usize>() + 22);
    let mut central = Vec::new();
    for (name, bytes) in files {
        let (name, bytes) = (name.as_ref().as_bytes(), bytes.as_ref());
        let offset = u32::try_from(out.len()).map_err(|_| too_big())?;
        let size = u32::try_from(bytes.len()).map_err(|_| too_big())?;
        let name_len = u16::try_from(name.len()).map_err(|_| format!("file name too long to zip: {}", String::from_utf8_lossy(name)))?;
        let mut crc = Crc::new();
        crc.update(bytes);
        // The fields both headers share: flags, method (0: stored), time, date, CRC, sizes, name length.
        let mut shared = Vec::with_capacity(26);
        for v in [UTF8_NAMES, 0, 0, DOS_DATE] {
            shared.extend_from_slice(&v.to_le_bytes());
        }
        for v in [crc.sum(), size, size] {
            shared.extend_from_slice(&v.to_le_bytes());
        }
        shared.extend_from_slice(&name_len.to_le_bytes());
        // Local file header.
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&shared);
        out.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        out.extend_from_slice(name);
        out.extend_from_slice(bytes);
        // Central directory record.
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&VERSION.to_le_bytes()); // made by
        central.extend_from_slice(&VERSION.to_le_bytes()); // needed to extract
        central.extend_from_slice(&shared);
        // Extra and comment lengths, disk number, internal and external attributes.
        central.extend_from_slice(&[0; 2 + 2 + 2 + 2 + 4]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
    }
    let start = u32::try_from(out.len()).map_err(|_| too_big())?;
    let central_len = u32::try_from(central.len()).map_err(|_| too_big())?;
    out.extend_from_slice(&central);
    // End of central directory.
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]); // this disk, the directory's disk
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&central_len.to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length
    Ok(out)
}

/// The entries of a store-only archive written by [`store`] (read through its central directory,
/// checking every CRC) → `(name, bytes)`.
#[cfg(test)]
pub fn entries(zip: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let u16_at = |at: usize| zip.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or("truncated");
    let u32_at = |at: usize| zip.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or("truncated");
    let end = zip.len().checked_sub(22).ok_or("no end record")?;
    if u32_at(end)? != 0x0605_4b50 {
        return Err("no end record".into());
    }
    let count = u16_at(end + 10)? as usize;
    let mut at = u32_at(end + 16)? as usize;
    let mut out = vec![];
    for _ in 0..count {
        if u32_at(at)? != 0x0201_4b50 || u16_at(at + 10)? != 0 {
            return Err("not a stored central record".into());
        }
        let (crc, size, name_len) = (u32_at(at + 16)?, u32_at(at + 20)? as usize, u16_at(at + 28)? as usize);
        let extra = u16_at(at + 30)? as usize + u16_at(at + 32)? as usize;
        let local = u32_at(at + 42)? as usize;
        let name = String::from_utf8(zip.get(at + 46..at + 46 + name_len).ok_or("truncated")?.to_vec()).map_err(|e| e.to_string())?;
        if u32_at(local)? != 0x0403_4b50 {
            return Err(format!("{name}: no local header"));
        }
        let data = local + 30 + u16_at(local + 26)? as usize + u16_at(local + 28)? as usize;
        let bytes = zip.get(data..data + size).ok_or("truncated")?.to_vec();
        let mut c = Crc::new();
        c.update(&bytes);
        if c.sum() != crc {
            return Err(format!("{name}: CRC mismatch"));
        }
        out.push((name, bytes));
        at += 46 + name_len + extra;
    }
    Ok(out)
}
