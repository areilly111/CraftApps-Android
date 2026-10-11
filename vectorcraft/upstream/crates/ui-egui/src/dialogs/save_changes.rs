//! Save Changes (closing a modified document): Save / Don't Save / Cancel. The flow lives in
//! [`crate::unsaved`]; this is its face in the shared dialog frame.

use super::DialogSpec;

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |d| crate::i18n::fmt(tl!("Do you want to save the changes you made to “{name}”?"), &[("name", &d.str("name"))]),
    body: |_, ui, _| {
        let t = crate::theme::Tokens::get(ui.ctx());
        ui.label(egui::RichText::new(tl!("Your changes will be lost if you don't save them.")).color(t.text_dim));
        false
    },
    confirm: |app, _| crate::unsaved::confirm(app),
    ok: Some("Save"),
    discard: Some("Don't Save"),
    // Long document names wrap instead of widening the dialog.
    max_width: Some(420.0),
    ..DialogSpec::FORM
};
