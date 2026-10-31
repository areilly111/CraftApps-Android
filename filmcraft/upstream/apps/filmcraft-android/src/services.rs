//! Android host services for FilmCraft's engine.
//!
//! Everything delegates to the engine's native [`FsServices`] except the
//! two things that are meaningless on a tablet: the Media Browser's start
//! directory and its drive list. With the desktop services the browser
//! starts at `$HOME` -- which is `/` for an Android app -- and cannot even
//! list it ("Cannot open /: Permission denied"). Here the browser starts
//! in the suite media folder, and its drives are Internal storage and
//! that same media folder: the places a tablet user actually keeps media.

use filmcraft_engine::media_browser::{DirEntry, Volume, VolumeKind};
use filmcraft_engine::{FsServices, Services};

/// Engine services for the Android port: [`FsServices`] with a tablet's
/// notion of "home" and "drives" for the Media Browser.
pub struct AndroidServices {
    inner: FsServices,
    /// Where the Media Browser starts: the suite media root.
    home: String,
}

impl AndroidServices {
    pub fn new(home: std::path::PathBuf) -> Self {
        Self { inner: FsServices, home: home.to_string_lossy().into_owned() }
    }
}

impl Services for AndroidServices {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
        self.inner.read_file(path)
    }

    fn write_file(&self, path: &str, data: &[u8]) -> std::io::Result<()> {
        self.inner.write_file(path, data)
    }

    fn write_file_new(&self, path: &str, data: &[u8]) -> std::io::Result<()> {
        self.inner.write_file_new(path, data)
    }

    fn file_size(&self, path: &str) -> std::io::Result<u64> {
        self.inner.file_size(path)
    }

    fn read_range(&self, path: &str, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        self.inner.read_range(path, offset, len)
    }

    fn reader(&self, path: &str) -> Option<std::io::Result<filmcraft_media::SharedReader>> {
        self.inner.reader(path)
    }

    fn list_dir(&self, dir: &str) -> Option<std::io::Result<Vec<String>>> {
        self.inner.list_dir(dir)
    }

    fn file_loader(&self) -> Option<filmcraft_media::sequence::FrameLoader> {
        self.inner.file_loader()
    }

    fn export_in_memory(&self) -> bool {
        self.inner.export_in_memory()
    }

    fn list_entries(&self, dir: &str) -> Option<std::io::Result<Vec<DirEntry>>> {
        self.inner.list_entries(dir)
    }

    fn volumes(&self) -> Vec<Volume> {
        vec![
            Volume { name: "Internal storage".into(), path: "/sdcard".into(), kind: VolumeKind::Local },
            Volume { name: "CraftApps Media".into(), path: self.home.clone(), kind: VolumeKind::Local },
        ]
    }

    fn home_dir(&self) -> Option<String> {
        Some(self.home.clone())
    }
}
