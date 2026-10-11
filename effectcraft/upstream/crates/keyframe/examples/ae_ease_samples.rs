//! Original scalar keyframe cases for comparison with AE through AEsync.
use effectcraft_keyframe::{Ease, Interp, Keyframe, Value, evaluate};
use effectcraft_time::Tick;
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let times: Vec<f64> = (0..=20).map(|i| f64::from(i) / 20.0).collect();
    let mut cases = Vec::new();
    for (name, from, to, out_speed, out_inf, in_speed, in_inf) in [
        ("easy", 0.0, 100.0, 0.0, 1.0 / 3.0, 0.0, 1.0 / 3.0),
        ("high_influence", 0.0, 100.0, 0.0, 0.8, 0.0, 0.8),
        ("asymmetric", 0.0, 100.0, 20.0, 0.8, 80.0, 0.6),
        ("overshoot", 0.0, 100.0, 400.0, 0.4, -100.0, 0.4),
        ("descending", 100.0, 0.0, -20.0, 0.6, -80.0, 0.8),
        ("equal_endpoints", 50.0, 50.0, 100.0, 0.4, -100.0, 0.4),
        ("maximum_influence", 0.0, 100.0, 0.0, 1.0, 0.0, 1.0),
        ("minimum_maximum", 0.0, 100.0, 0.0, 0.001, 0.0, 1.0),
    ] {
        let mut a = Keyframe::new(Tick::ZERO, Value::Scalar(from));
        let mut b = Keyframe::new(Tick::from_seconds_f64(1.0), Value::Scalar(to));
        a.out_interp = Interp::Bezier;
        b.in_interp = Interp::Bezier;
        a.out_ease = vec![Ease { speed: out_speed, influence: out_inf }];
        b.in_ease = vec![Ease { speed: in_speed, influence: in_inf }];
        let keys = [a, b];
        let values: Vec<f64> = times
            .iter()
            .map(|t| evaluate(&keys, Tick::from_seconds_f64(*t), false).map(|v| v.as_f64()).ok_or("missing keyframe evaluation"))
            .collect::<Result<_, _>>()?;
        cases.push(json!({"name":name,"from":from,"to":to,"outSpeed":out_speed,"outInfluence":out_inf * 100.0,"inSpeed":in_speed,"inInfluence":in_inf * 100.0,"times":times,"values":values}));
    }
    println!("{}", serde_json::to_string_pretty(&json!({"cases":cases}))?);
    Ok(())
}
