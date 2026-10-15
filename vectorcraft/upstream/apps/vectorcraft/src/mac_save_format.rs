//! Format menu on macOS save sheets (#866). rfd's Cocoa backend combines the extensions of
//! every filter but doesn't show their names. The accessory uses the same filters and updates
//! the filename, whose extension already determines the format in the save command.
//!
//! AppKit owns the accessory until the sheet closes; its target is the accessory itself. The
//! panel reference is weak, so closing the sheet releases both without a retain cycle.
//! Native view handles and Objective-C subclassing require the scoped unsafe allowance.
#![allow(unsafe_code)]

use std::path::Path;

use objc2::rc::{Retained, Weak};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSPopUpButton, NSSavePanel, NSTextField, NSView};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use vectorcraft_ui_egui::{FilePick, i18n};

type Filter = (&'static str, &'static [&'static str]);

struct Ivars {
    panel: Weak<NSSavePanel>,
    filters: Vec<Filter>,
}

define_class!(
    // SAFETY: NSView has no extra subclassing requirements; the subclass implements no Drop.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    #[name = "VectorcraftSaveFormatAccessory"]
    struct FormatAccessory;

    // SAFETY: NSObjectProtocol has no additional requirements.
    unsafe impl NSObjectProtocol for FormatAccessory {}

    impl FormatAccessory {
        // SAFETY: AppKit's target/action sends the popup as the single object argument.
        #[unsafe(method(chooseFormat:))]
        fn choose_format(&self, sender: &NSPopUpButton) {
            let Some(panel) = self.ivars().panel.load() else { return };
            let Some(index) = usize::try_from(sender.indexOfSelectedItem()).ok() else { return };
            let Some((_, extensions)) = self.ivars().filters.get(index) else { return };
            let Some(extension) = extensions.first() else { return };
            let name = panel.nameFieldStringValue().to_string();
            let new = filename(&name, extension, &self.ivars().filters);
            panel.setNameFieldStringValue(&NSString::from_str(&new));
        }
    }
);

/// The suggested file's type, or the request's default (the first filter).
fn initial_format(name: &str, filters: &[Filter]) -> usize {
    let extension = Path::new(name).extension().and_then(|e| e.to_str());
    filters.iter().position(|(_, extensions)| extension.is_some_and(|e| extensions.iter().any(|x| e.eq_ignore_ascii_case(x)))).unwrap_or(0)
}

/// Replace a known format suffix; keep other dots (e.g. a version number) as part of the name.
fn filename(name: &str, extension: &str, filters: &[Filter]) -> String {
    if name.is_empty() {
        return name.to_owned();
    }
    let path = Path::new(name);
    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| filters.iter().any(|(_, extensions)| extensions.iter().any(|x| e.eq_ignore_ascii_case(x))))
    {
        path.with_extension(extension).to_string_lossy().into_owned()
    } else {
        format!("{name}.{extension}")
    }
}

/// Add a Format menu to the save sheet rfd just began on `window`. Open panels and requests
/// without several named file types are left to rfd.
pub fn attach(window: &winit::window::Window, pick: &FilePick) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let filters: Vec<_> = pick.filters.iter().copied().filter(|(_, extensions)| !extensions.is_empty() && !extensions.contains(&"*")).collect();
    if filters.len() < 2 {
        return;
    }
    let Ok(handle) = window.window_handle() else { return };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else { return };
    // SAFETY: winit keeps its content view alive while this window reference is in use.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let Some(sheet) = view.window().and_then(|w| w.attachedSheet()) else { return };
    let Some(panel) = sheet.downcast_ref::<NSSavePanel>() else { return };
    let selected = initial_format(&pick.name, &filters);
    let this = FormatAccessory::alloc(mtm).set_ivars(Ivars { panel: Weak::new(panel), filters });
    // SAFETY: NSView's designated initializer is called once on the allocated subclass.
    let accessory: Retained<FormatAccessory> = unsafe { msg_send![super(this), initWithFrame: NSRect::new(NSPoint::ZERO, NSSize::new(360.0, 38.0))] };
    let label = NSTextField::labelWithString(&NSString::from_str(i18n::t("Format:")), mtm);
    label.setFrame(NSRect::new(NSPoint::new(0.0, 10.0), NSSize::new(80.0, 20.0)));
    let popup =
        NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), NSRect::new(NSPoint::new(82.0, 6.0), NSSize::new(278.0, 28.0)), false);
    for (name, _) in &accessory.ivars().filters {
        popup.addItemWithTitle(&NSString::from_str(i18n::t(name)));
    }
    // The filter list comes from the built-in format registry, small enough for NSInteger.
    if let Ok(index) = isize::try_from(selected) {
        popup.selectItemAtIndex(index);
    }
    // SAFETY: the accessory owns the popup and AppKit keeps the accessory while the sheet is
    // open. The selector above has exactly the target/action signature expected by NSControl.
    unsafe {
        popup.setTarget(Some(&accessory));
        popup.setAction(Some(sel!(chooseFormat:)));
        accessory.addSubview(&label);
        accessory.addSubview(&popup);
    }
    panel.setExtensionHidden(false);
    panel.setAccessoryView(Some(&accessory));
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILTERS: &[Filter] = &[("VectorCraft", &["vectorcraft", "drawcraft"]), ("SVG", &["svg"]), ("PDF", &["pdf"])];

    #[test]
    fn current_suffix_selects_the_format_and_an_unknown_one_uses_the_default() {
        for (name, index) in [("Untitled-1", 0), ("旧形式.drawcraft", 0), ("a b.SVG", 1), ("art.pdf", 2), ("version.2", 0)] {
            assert_eq!(initial_format(name, FILTERS), index);
        }
    }

    #[test]
    fn changing_format_keeps_the_name_and_avoids_duplicate_extensions() {
        for (name, expected) in
            [("図面 1.vectorcraft", "図面 1.svg"), ("a.PDF", "a.svg"), ("art", "art.svg"), ("art.v2", "art.v2.svg"), ("art.svg", "art.svg"), ("", "")]
        {
            assert_eq!(filename(name, "svg", FILTERS), expected);
        }
    }
}
