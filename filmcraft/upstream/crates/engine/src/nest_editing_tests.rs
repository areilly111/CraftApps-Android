//! Editing with nested sequences: the nest toggle ("Insert and overwrite sequences as nests or
//! individual clips"), a sequence as a source, Match Frame on a nest, and Reveal in Project.
//! Premiere's behaviour was observed in Premiere Pro 26.5.2.

use super::*;
use filmcraft_project::{TrackItem, TrackKind};
use filmcraft_time::TimeRange;
use serde_json::json;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn clips(s: &Session, kind: TrackKind, idx: usize) -> Vec<TrackItem> {
    s.active_sequence().unwrap().tracks(kind)[idx].items.clone()
}

/// A subsequence of the first three V1 clips of the demo (with their sound and the transitions
/// between them), and a new empty sequence, which is left active.
fn source_and_empty(s: &mut Session) -> (ItemId, ItemId) {
    let v1: Vec<u64> = clips(s, TrackKind::Video, 0).iter().take(3).map(|c| c.id.0).collect();
    s.execute("timeline.select", json!({"clips": v1})).unwrap();
    let source = ItemId(s.execute("sequence.makeSubsequence", json!({"name": "Source"})).unwrap()["sequence"].as_u64().unwrap());
    let empty = ItemId(s.execute("file.newSequence", json!({"name": "Target"})).unwrap()["sequence"].as_u64().unwrap());
    s.execute("sequence.open", json!({"item": empty.0})).unwrap();
    (source, empty)
}

#[test]
fn the_nest_toggle_is_on_by_default_and_can_be_set() {
    let mut s = demo();
    assert!(!s.state.sequences_as_clips);
    assert_eq!(s.execute("sequence.nestSequences", json!({})).unwrap()["nest"], false);
    assert!(s.state.sequences_as_clips);
    assert_eq!(s.execute("sequence.nestSequences", json!({})).unwrap()["nest"], true);
    assert_eq!(s.execute("sequence.nestSequences", json!({"on": false})).unwrap()["nest"], false);
    assert_eq!(s.execute("sequence.nestSequences", json!({"on": false})).unwrap()["nest"], false);
    assert!(s.state.sequences_as_clips);
}

#[test]
fn with_the_toggle_on_a_sequence_edits_in_as_one_nest() {
    let mut s = demo();
    let (source, _) = source_and_empty(&mut s);
    let r = s.execute("timeline.place", json!({"item": source.0, "seconds": 0.0})).unwrap();
    assert_eq!(r["clips"].as_array().unwrap().len(), 2, "one picture and one sound clip");
    let v1 = clips(&s, TrackKind::Video, 0);
    assert_eq!(v1.len(), 1);
    assert_eq!(v1[0].item, source);
}

#[test]
fn with_the_toggle_off_a_sequence_edits_in_as_its_clips_with_their_transitions() {
    let mut s = demo();
    let (source, _) = source_and_empty(&mut s);
    let src = s.project.sequence(source).unwrap().clone();
    let (src_v, src_a) = (src.video_tracks[0].clone(), src.audio_tracks[0].clone());
    assert!(src_v.items.len() == 3 && !src_v.transitions.is_empty(), "the source has transitions to carry");
    s.execute("sequence.nestSequences", json!({"on": false})).unwrap();
    let undo = s.history.undo.len();
    let at = s.sequence_rate().tick_of(48);
    s.execute("timeline.place", json!({"item": source.0, "time": at.0})).unwrap();
    assert_eq!(s.history.undo.len(), undo + 1, "one undo step");
    let q = s.active_sequence().unwrap();
    q.check().unwrap();
    let (v, a) = (&q.video_tracks[0], &q.audio_tracks[0]);
    // the same clips, at the same places from `at`, as copies with ids of their own
    assert_eq!(v.items.len(), 3);
    for (new, old) in v.items.iter().zip(&src_v.items).chain(a.items.iter().zip(&src_a.items)) {
        assert_eq!((new.item, new.start, new.duration, new.source_in), (old.item, old.start + at, old.duration, old.source_in));
        assert_ne!(new.id, old.id);
    }
    assert!(v.items.iter().all(|i| s.project.sequence(i.item).is_none()), "no nest");
    // picture and sound stay linked pair by pair, with links of their own
    for (pic, snd) in v.items.iter().zip(&a.items) {
        assert!(pic.link.is_some() && pic.link == snd.link);
    }
    assert!(v.items.iter().zip(&src_v.items).all(|(n, o)| n.link != o.link));
    // the transitions came along, between the copies
    assert_eq!((v.transitions.len(), a.transitions.len()), (src_v.transitions.len(), src_a.transitions.len()));
    for (new, old) in v.transitions.iter().zip(&src_v.transitions) {
        assert_eq!((new.start, new.duration), (old.start + at, old.duration));
        assert!(new.id != old.id && new.from.is_none_or(|c| v.item(c).is_some()) && new.to.is_none_or(|c| v.item(c).is_some()));
    }
    // the copies are what is selected, and the source sequence is untouched
    assert_eq!(s.state.selection.len(), v.items.len() + a.items.len());
    assert_eq!(s.project.sequence(source).unwrap(), &src);
}

#[test]
fn source_tracks_with_clips_go_to_consecutive_tracks_and_missing_tracks_are_added() {
    let mut s = demo();
    let (source, target) = source_and_empty(&mut s);
    // in the source, put the third clip's picture on V3 (V2 stays empty)
    s.execute("sequence.open", json!({"item": source.0})).unwrap();
    let third = clips(&s, TrackKind::Video, 0)[2].clone();
    s.execute("timeline.move", json!({"moves": [{"clip": third.id.0, "track": "V3", "time": third.start.0}], "linked": false})).unwrap();
    assert_eq!(s.active_sequence().unwrap().video_tracks[2].items.len(), 1);
    s.execute("sequence.open", json!({"item": target.0})).unwrap();
    s.execute("sequence.nestSequences", json!({"on": false})).unwrap();
    // dropped on V1: source V1 and V3 land on V1 and V2
    s.execute("timeline.place", json!({"item": source.0, "track": "V1", "seconds": 0.0})).unwrap();
    let q = s.active_sequence().unwrap();
    assert_eq!((q.video_tracks[0].items.len(), q.video_tracks[1].items.len(), q.video_tracks[2].items.len()), (2, 1, 0));
    assert_eq!(q.video_tracks[1].items[0].item, third.item);
    // dropped on the top track, V3: the second lane needs a V4, which is added
    assert_eq!(q.video_tracks.len(), 3);
    s.execute("timeline.place", json!({"item": source.0, "track": "V3", "seconds": 60.0})).unwrap();
    let q = s.active_sequence().unwrap();
    q.check().unwrap();
    assert_eq!(q.video_tracks.len(), 4);
    assert_eq!((q.video_tracks[2].items.len(), q.video_tracks[3].items.len()), (2, 1));
    assert_eq!(q.video_tracks[3].name, "Video 4");
    // one undo removes the clips and the track again
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active_sequence().unwrap().video_tracks.len(), 3);
}

#[test]
fn from_the_source_monitor_the_marked_part_of_a_sequence_is_edited_in() {
    let mut s = demo();
    let (source, _) = source_and_empty(&mut s);
    let rate = s.sequence_rate();
    let src_v = s.project.sequence(source).unwrap().video_tracks[0].clone();
    // In inside the first clip, Out inside the second
    let (mark_in, mark_out) = (src_v.items[0].start + rate.tick_of(12), src_v.items[1].start + rate.tick_of(23));
    s.execute("source.open", json!({"item": source.0})).unwrap();
    s.execute("project.setMarks", json!({"item": source.0, "in": mark_in.0, "out": mark_out.0})).unwrap();
    let marked = TimeRange::from_bounds(mark_in, mark_out + rate.frame_duration());

    // as a nest: one clip showing that part of the sequence
    s.execute("source.overwrite", json!({})).unwrap();
    let nest = clips(&s, TrackKind::Video, 0)[0].clone();
    assert_eq!((nest.item, nest.source_in, nest.duration), (source, marked.start, marked.duration));
    s.execute("edit.undo", json!({})).unwrap();

    // as clips: the two clips the marks cut through, trimmed to them
    s.execute("sequence.nestSequences", json!({"on": false})).unwrap();
    s.set_playhead(Tick::ZERO);
    s.execute("source.overwrite", json!({})).unwrap();
    let v = clips(&s, TrackKind::Video, 0);
    assert_eq!(v.len(), 2);
    assert_eq!((v[0].start, v[0].source_in), (Tick::ZERO, src_v.items[0].source_in + rate.tick_of(12)));
    assert_eq!(v[0].end(), v[1].start);
    assert_eq!(v[1].end(), marked.duration);
    assert_eq!(v[1].source_in, src_v.items[1].source_in);
    s.active_sequence().unwrap().check().unwrap();
}

#[test]
fn a_sequence_can_be_edited_into_itself_as_clips_but_not_as_a_nest() {
    let mut s = demo();
    let main = s.state.active_sequence.unwrap();
    let before = clips(&s, TrackKind::Video, 0).len();
    assert!(s.execute("timeline.place", json!({"item": main.0, "seconds": 120.0})).is_err());
    s.execute("sequence.nestSequences", json!({"on": false})).unwrap();
    s.execute("timeline.place", json!({"item": main.0, "seconds": 120.0})).expect("its clips are just clips");
    assert_eq!(clips(&s, TrackKind::Video, 0).len(), before * 2);
    assert_eq!(s.project.nest_cycle(), None);
}

#[test]
fn match_frame_on_a_nest_loads_the_nested_sequence_at_the_frame_it_shows() {
    let mut s = demo();
    let rate = s.sequence_rate();
    let v = clips(&s, TrackKind::Video, 0)[1].id;
    s.execute("timeline.select", json!({"clips": [v.0]})).unwrap();
    let r = s.execute("clip.nest", json!({"name": "Inner"})).unwrap();
    let (nested, nest) = (ItemId(r["sequence"].as_u64().unwrap()), ClipId(r["clips"][0].as_u64().unwrap()));
    let clip = |s: &Session| s.active_sequence().unwrap().find_item(nest).unwrap().1.clone();
    // plain: 9 frames into the nest is 9 frames into its sequence
    s.set_playhead(clip(&s).start + rate.tick_of(9));
    s.execute("sequence.matchFrame", json!({})).unwrap();
    assert_eq!((s.state.source_item, s.state.source_playhead), (Some(nested), rate.tick_of(9)));
    // trimmed in by 5 frames: the same timeline frame now shows frame 9 still, 4 frames into the clip
    s.execute("timeline.trim", json!({"clip": nest.0, "edge": "in", "deltaFrames": 5})).unwrap();
    s.set_playhead(clip(&s).start + rate.tick_of(4));
    s.execute("sequence.matchFrame", json!({})).unwrap();
    assert_eq!(s.state.source_playhead, rate.tick_of(9));
    // at double speed, 4 frames in is 8 frames further into the sequence
    s.execute("timeline.select", json!({"clips": [nest.0]})).unwrap();
    s.execute("clip.speedDuration", json!({"speed": 200.0})).unwrap();
    s.set_playhead(clip(&s).start + rate.tick_of(4));
    s.execute("sequence.matchFrame", json!({})).unwrap();
    assert_eq!((s.state.source_item, s.state.source_playhead), (Some(nested), rate.tick_of(5 + 8)));
}

#[test]
fn reveal_in_project_selects_the_clips_item_and_asks_the_ui_to_show_it() {
    let mut s = demo();
    let v1 = clips(&s, TrackKind::Video, 0);
    s.drain_events();
    // the selected clip
    s.execute("timeline.select", json!({"clips": [v1[2].id.0]})).unwrap();
    assert_eq!(s.execute("clip.revealInProject", json!({})).unwrap()["item"], v1[2].item.0);
    assert_eq!(s.state.project_selection, vec![v1[2].item]);
    assert!(s.drain_events().iter().any(|e| matches!(e, Event::RevealInProject(i) if *i == v1[2].item)));
    // a named clip
    s.execute("clip.revealInProject", json!({"clip": v1[0].id.0})).unwrap();
    assert_eq!(s.state.project_selection, vec![v1[0].item]);
    // nothing selected: the clip under the playhead on a targeted track
    s.execute("timeline.select", json!({"clips": []})).unwrap();
    s.set_playhead(v1[3].start + Tick(v1[3].duration.0 / 2));
    s.execute("clip.revealInProject", json!({})).unwrap();
    assert_eq!(s.state.project_selection, vec![v1[3].item]);
    // the sequence in the Timeline
    let main = s.state.active_sequence.unwrap();
    s.execute("sequence.revealInProject", json!({})).unwrap();
    assert_eq!(s.state.project_selection, vec![main]);
    assert!(s.drain_events().iter().any(|e| matches!(e, Event::RevealInProject(i) if *i == main)));
}

#[test]
fn mark_in_and_out_in_the_source_monitor_mark_a_sequence_like_a_clip() {
    // marks on a sequence loaded in the Source Monitor used to be dropped silently
    let mut s = demo();
    let (source, _) = source_and_empty(&mut s);
    let rate = s.sequence_rate();
    s.execute("source.open", json!({"item": source.0})).unwrap();
    s.execute("markers.markIn", json!({"target": "source", "time": rate.tick_of(10).0})).unwrap();
    s.execute("markers.markOut", json!({"target": "source", "time": rate.tick_of(40).0})).unwrap();
    let q = s.project.sequence(source).unwrap();
    assert_eq!((q.mark_in, q.mark_out), (Some(rate.tick_of(10)), Some(rate.tick_of(40))));
    // and can be cleared again
    s.execute("project.setMarks", json!({"item": source.0, "in": null, "out": null})).unwrap();
    let q = s.project.sequence(source).unwrap();
    assert_eq!((q.mark_in, q.mark_out), (None, None));
}
