//! Little-endian values: the buffer both writers build records in, and a reader that never reads
//! past its data (every read is an `Option`).

/// Bytes being written.
#[derive(Default)]
pub(crate) struct Out(pub Vec<u8>);

impl Out {
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i16(&mut self, v: i16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
    }
    /// Zeros up to the next multiple of `n` bytes.
    pub fn pad(&mut self, n: usize) {
        while !self.0.len().is_multiple_of(n) {
            self.0.push(0);
        }
    }
    /// Overwrite the `u32` at `at` (a size or count known only later).
    pub fn set_u32(&mut self, at: usize, v: u32) {
        if let Some(b) = self.0.get_mut(at..at + 4) {
            b.copy_from_slice(&v.to_le_bytes());
        }
    }
    pub fn set_i32(&mut self, at: usize, v: i32) {
        if let Some(b) = self.0.get_mut(at..at + 4) {
            b.copy_from_slice(&v.to_le_bytes());
        }
    }
}

/// Sequential little-endian reads from a slice.
#[derive(Clone, Copy)]
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    pub fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let b = self.data.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(b)
    }
    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.bytes(N)?.try_into().ok()
    }
    pub fn u16(&mut self) -> Option<u16> {
        self.array().map(u16::from_le_bytes)
    }
    pub fn i16(&mut self) -> Option<i16> {
        self.array().map(i16::from_le_bytes)
    }
    pub fn u32(&mut self) -> Option<u32> {
        self.array().map(u32::from_le_bytes)
    }
    pub fn i32(&mut self) -> Option<i32> {
        self.array().map(i32::from_le_bytes)
    }
    pub fn f32(&mut self) -> Option<f32> {
        self.array().map(f32::from_le_bytes)
    }
    pub fn skip(&mut self, n: usize) -> Option<()> {
        self.bytes(n).map(drop)
    }
    /// The bytes not read yet.
    pub fn rest(&self) -> &'a [u8] {
        self.data.get(self.pos..).unwrap_or_default()
    }
}

/// `len` bytes of `data` from `offset` (record-relative offsets read from untrusted files).
pub(crate) fn slice(data: &[u8], offset: u32, len: u32) -> Option<&[u8]> {
    let start = usize::try_from(offset).ok()?;
    data.get(start..start.checked_add(usize::try_from(len).ok()?)?)
}
