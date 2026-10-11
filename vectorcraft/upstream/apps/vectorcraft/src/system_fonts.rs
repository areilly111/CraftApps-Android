//! Windows: the font files in DirectWrite's system font collection, scanned with the font folders
//! (`vectorcraft_text::set_platform_font_files`). The desktop app and `vectorcraft-cli` (exports,
//! MCP) both install it; the CLI shares this file.
//!
//! Font services such as Adobe Fonts load their fonts with GDI at run time, from files in their
//! own folders, without installing them: the font folders and the registry don't list them. Since
//! Windows 10 (version 1607) DirectWrite's system font collection holds them too
//! (<https://learn.microsoft.com/windows/win32/directwrite/what-s-new-in-directwrite-for-windows-8-consumer-preview>,
//! "Support for Adobe Typekit and other font-service clients"), so VectorCraft lists them as other
//! apps do, reading the files in place (#579).
//!
//! DirectWrite is COM, which has no safe API, hence the scoped `unsafe_code` allowance.
#![allow(unsafe_code)]

use std::collections::BTreeSet;

use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWriteCreateFactory, IDWriteFactory3, IDWriteFontSet, IDWriteLocalFontFileLoader,
};
use windows::core::Interface;

/// The most fonts read from the collection (a system has a few thousand).
const MAX_FONTS: u32 = 1 << 16;
/// The longest file path taken, in UTF-16 units (Windows' own limit).
const MAX_PATH: u32 = 32_767;

/// Have the font scans read the font files DirectWrite lists.
pub fn install() {
    vectorcraft_text::set_platform_font_files(font_files);
}

/// The local files of the fonts in DirectWrite's system font collection as it is now (fonts
/// loaded since the last call included). None before Windows 10, or when DirectWrite fails: the
/// font folders and the registry still list the installed fonts.
fn font_files() -> Vec<String> {
    match collection_files() {
        Ok(files) => files.into_iter().collect(),
        Err(e) => {
            log::debug!("DirectWrite's system fonts aren't listed: {e}");
            Vec::new()
        }
    }
}

fn collection_files() -> windows::core::Result<BTreeSet<String>> {
    // SAFETY: plain DirectWrite calls on interfaces it returned, each checked for failure; the
    // collection is asked to check for fonts loaded since the factory last looked.
    let set = unsafe {
        let factory: IDWriteFactory3 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let mut collection = None;
        factory.GetSystemFontCollection(false, &mut collection, true)?;
        let Some(collection) = collection else { return Ok(BTreeSet::new()) };
        collection.GetFontSet()?
    };
    // SAFETY: a count of the set's fonts.
    let count = unsafe { set.GetFontCount() }.min(MAX_FONTS);
    // Faces of one file (a collection's, a variable font's instances) name it once.
    Ok((0..count).filter_map(|i| local_path(&set, i)).collect())
}

/// The path of the file holding font `i` of `set`, when it is a local file (not a downloadable
/// font, nor one in memory).
fn local_path(set: &IDWriteFontSet, i: u32) -> Option<String> {
    // SAFETY: `i` is below the set's count; the key DirectWrite hands back stays valid while
    // `file` (which owns it) lives, and is passed back with its own size; the path buffer is
    // sized from DirectWrite's answer plus the terminating NUL.
    unsafe {
        let file = set.GetFontFaceReference(i).ok()?.GetFontFile().ok()?;
        let loader: IDWriteLocalFontFileLoader = file.GetLoader().ok()?.cast().ok()?;
        let (mut key, mut size) = (std::ptr::null_mut(), 0u32);
        file.GetReferenceKey(&mut key, &mut size).ok()?;
        let len = loader.GetFilePathLengthFromKey(key, size).ok()?;
        if len == 0 || len > MAX_PATH {
            return None;
        }
        let mut path = vec![0u16; len as usize + 1];
        loader.GetFilePathFromKey(key, size, &mut path).ok()?;
        Some(String::from_utf16_lossy(path.get(..len as usize)?))
    }
}

#[cfg(test)]
mod tests {
    /// DirectWrite lists the installed fonts, by their files' full paths.
    #[test]
    fn directwrite_lists_the_installed_font_files() {
        let files = super::font_files();
        assert!(files.iter().all(|f| std::path::Path::new(f).is_absolute()), "{:?}", files.first());
        let fonts = std::path::Path::new(&std::env::var("WINDIR").unwrap()).join("Fonts").to_string_lossy().to_lowercase();
        assert!(files.iter().any(|f| f.to_lowercase().starts_with(&fonts)), "{} files, {:?}", files.len(), files.first());
    }
}
