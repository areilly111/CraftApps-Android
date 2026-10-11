# Measured keyframe fidelity

The [AE 2026 replacement acceptance gates](ae-2026-acceptance.md) define the broader
target across workflows, rendered output, work-project compatibility, performance and platforms.

The first G1 comparison measures eight original scalar Rotation cases against a live After Effects
2026 session through AEsync 2.0.4: Easy Ease, overlapping influences, asymmetric ease, overshoot,
descending values, equal endpoints, 100%/100% influence and 0.1%/100% influence. Each case has
21 samples over a one-second segment. This is a scalar temporal-interpolation measurement, not an
overall animation, rendering, spatial-path or application-parity score.

## Reproduce

With Node.js, Rust and an installed AEsync CLI, open a scratch After Effects project and run from
the EffectCraft checkout:

```sh
node examples/check-ae-ease.mjs /path/to/aesync/server/dist/cli.js
```

The external bridge must be answering. Signing into AEsync's embedded chat is not required.
The script creates a temporary comp with an original text layer, samples its Rotation property and
removes the comp on success or failure. It does not save, close or replace the current project.
Run in a scratch project: creating/removing the comp still changes the dirty flag and undo history.

`crates/keyframe/examples/ae_ease_samples.rs` evaluates the corresponding EffectCraft keys.
`examples/ae-ease-oracle.jsx` records live property samples and actual ease settings after the setters.
`examples/check-ae-ease.mjs` checks case metadata, applied AE settings, finite values and sample times,
then compares every value with an absolute tolerance of `1e-7` degrees. A failing score exits nonzero.

Raw host observations and EffectCraft samples stay in gitignored `plan/aftereffects/ref/ease/`.
Only the original harness and aggregate scores in `keyframe-ease.json` are tracked. No Adobe artwork,
presets, screenshots, frames or implementation code are included.

## Finding and validation

Measured on 5 October 2026 with After Effects `26.3x87` and AEsync 2.0.4. Before the fix,
EffectCraft normalized outgoing/incoming influence when their sum exceeded 100%. AE retains each
influence. That normalization caused maximum errors of 12.4581 degrees for 80%/80%, 8.8909 degrees
for the asymmetric case and 5.5636 degrees for the descending case. Removing normalization in both
scalar evaluation and the shared progress helper makes all eight cases pass; the largest difference
in the 168 samples is below `3e-11` degrees.

The numeric regression test checks AE-derived behavioral facts, including the formerly failing
sample at 0.1 seconds (0.59243939036327 degrees; previously 1.4621885963370382). Existing linear,
hold, spatial, roving and eased-range tests remain green.

Astra review also found premature Newton convergence near the flat midpoint of the 100%/100%
time curve. Its closed-form inverse and a parameter-based convergence check now have a separate
mathematical regression test at times within `1e-12` of the midpoint; that test is additional to
the 168 live AE samples.

## Still unmeasured

Rendered frame comparisons now have a deterministic `effectcraft_effects::fidelity::compare_frames`
helper. Corpus runners can use it with locally generated host/reference frames and fail on either a
mean or maximum channel error without committing proprietary renders.

Mixed interpolation sides, non-unit durations, automatic/continuous Bezier, spatial speeds and paths,
time remapping, expressions, actual rendered motion and other effects need separate corpus cases.
The initial probe makes no claim about those features.
