use effectcraft_time::{FrameRate, parse_timecode};

const RELATIVE_NEGATIVE_MIN_FIELD: &str = "-0:0:0:-9223372036854775808";

#[test]
fn normal_relative_and_current_zero_controls_keep_existing_results() {
    let rate = FrameRate::FPS_30;
    assert_eq!(parse_timecode("+15", rate, false, 100).unwrap(), 115);
    assert_eq!(parse_timecode("-1.00", rate, false, 100).unwrap(), 70);
    assert_eq!(parse_timecode("-1", rate, false, i64::MIN).unwrap(), i64::MIN);
    assert_eq!(parse_timecode(RELATIVE_NEGATIVE_MIN_FIELD, rate, false, 0).unwrap(), i64::MAX);
    assert_eq!(parse_timecode("+0:0:0:-9223372036854775808", rate, false, 0).unwrap(), i64::MIN);
}

#[test]
fn relative_negative_min_field_preserves_cancellation_from_negative_one() {
    let actual = parse_timecode(RELATIVE_NEGATIVE_MIN_FIELD, FrameRate::FPS_30, false, -1).unwrap();
    eprintln!("relative MIN field: current=-1 actual={actual} expected={}", i64::MAX);
    // The relative offset is positive 2^63; minus one is exactly i64::MAX.
    assert_eq!(actual, i64::MAX, "saturation must preserve cancellation with the current frame");
}

#[test]
fn relative_negative_min_field_cancels_the_minimum_current_frame() {
    let actual = parse_timecode(RELATIVE_NEGATIVE_MIN_FIELD, FrameRate::FPS_30, false, i64::MIN).unwrap();
    eprintln!("relative MIN field: current=MIN actual={actual} expected=0");
    // The starting frame is negative 2^63; the relative offset is positive 2^63.
    assert_eq!(actual, 0, "opposite extreme relative offsets must cancel before saturation");
}
