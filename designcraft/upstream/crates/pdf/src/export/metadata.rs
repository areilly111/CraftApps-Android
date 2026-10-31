use super::{Document, Metadata, krilla};

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn now_unix() -> Option<i64> {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs() as i64)
}

#[cfg(target_arch = "wasm32")]
pub(super) fn now_unix() -> Option<i64> {
    None
}

/// The document information from File Info (title, author, description, keywords). `title` and
/// `author` from the export options replace File Info's.
pub(super) fn file_info_metadata(doc: &Document, title: Option<&str>, author: Option<&str>) -> Metadata {
    let info = &doc.file_info;
    let description = info.description.trim();
    Metadata {
        creator: "DesignCraft".into(),
        producer: "DesignCraft".into(),
        title: Some(title.unwrap_or_else(|| doc.metadata_title()).to_string()),
        authors: author.or_else(|| doc.metadata_author()).map(str::to_string).into_iter().collect(),
        description: (!description.is_empty()).then(|| description.to_string()),
        keywords: info.keywords.clone(),
        created: None,
        modified: None,
    }
}

pub(super) fn date_time(t: i64) -> krilla::metadata::DateTime {
    let c = civil(t);
    krilla::metadata::DateTime::new(c.0.clamp(0, 9999) as u16)
        .month(c.1)
        .day(c.2)
        .hour(c.3)
        .minute(c.4)
        .second(c.5)
        .utc_offset_hour(0)
        .utc_offset_minute(0)
}

/// Unix seconds (UTC) → (year, month, day, hour, minute, second), proleptic Gregorian. A time
/// outside chrono's year range still converts (the calendar repeats every 400 years, so the
/// instant is moved into range by whole cycles and only the year is carried, matching the old
/// `civil`), rather than resetting to the epoch. `date_time` applies the PDF `0..=9999` year clamp.
pub(crate) fn civil(t: i64) -> (i64, u8, u8, u8, u8, u8) {
    use chrono::{Datelike, Timelike};
    let (y, t) = designcraft_doc::time::utc_parts(t, 0);
    (y, t.month() as u8, t.day() as u8, t.hour() as u8, t.minute() as u8, t.second() as u8)
}
