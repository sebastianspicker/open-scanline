# Performance checks

Timing numbers are easy to misread. This page describes the reproducible runner,
what it records, and where the limits are. It is aimed at contributors changing
hot paths, not at end users.

Use the pinned Rust 1.91.0 toolchain and locked dependencies. Run from the
repository root, with other builds and heavyweight applications stopped:

```sh
python3 scripts/run_performance.py --output target/optimization/current.json
```

The runner builds the release example before it measures. Inputs use seed
1592594996, sizes 1000 × 1000 and 6000 × 4000, and Gray8, RGB8, and RGBA8 packed
pixels. Every workload has three warmups and ten measured repetitions.

The JSON records each latency in nanoseconds, allocation calls, cumulative
requested allocation bytes, process peak RSS where available, compiler, platform,
feature selection, revision, dirty state, Rust/Cargo source fingerprint, and
benchmark binary size. Write output under the ignored `target/` directory; it
contains no scanned documents or model weights.

## Workloads

The default workloads cover the borrowed default pipeline, 180° and 270°
rotations, and a radius-two box blur. Add `--median` for the more expensive
radius-one median workload. Add `--media` to also measure direct RGB conversion,
BMP/JPEG encoding, and scanner-profile application. Use `--features ocrs,onnx` to
build the example with full headless support. Use
`--operations rotate180,box_blur,scanner_profile` to investigate selected
workloads without repeating unrelated encoders. `--skip-build --binary PATH`
measures a previously built executable, including an isolated baseline built from
the same workload source.

## Comparing results

Compare the same workload, pixel format, dimensions, machine, compiler, build
profile, and repetition count. Report the median and spread of all samples; do not
select favorable runs.

- Allocation bytes are cumulative traffic, not simultaneously live memory.
- Peak RSS covers the entire benchmark process, including fixture construction
  and allocator retention, so it does not identify the peak of a single operation.
- Record executable sizes from the same release settings, separately for each
  feature profile.

Correctness and structural resource checks run in ordinary tests. Timing
thresholds are deliberately excluded from CI, because scheduling, CPU scaling,
filesystem caches, and host load all affect latency. Mock and file acquisition
provide hardware-free checks. Injected OCR engines demonstrate job reuse and model
verification, but do not verify recognition quality or real model load time. The
optional `OPEN_SCANLINE_OCR_REAL_MODEL_TEST_DIR` fixture remains an explicit local
opt-in; no part of this workflow downloads a model.

## Focused benchmarks

### ONNX tensor fill

An ignored ONNX unit benchmark compares the retained pre-optimization oracle with
the direct path and emits JSON for a 224 × 224 RGB target:

```sh
cargo test --release --no-default-features --features onnx --locked --lib \
  release_benchmark_emits_json_for_one_and_twenty_four_megapixels \
  -- --ignored --nocapture --test-threads=1
```

Run it after compilation has finished, without competing builds. The ordinary
allocation test requires exactly one allocation equal to the final tensor size,
including for a 24 MP source. Numerical tests keep the historical one-channel
layout behavior: NCHW computes weighted luma, while NHWC selects red. That
compatibility rule is intentional.

### Median-neighborhood selection

```sh
cargo run --release --no-default-features --locked --example median_selection \
  > target/optimization/median-selection.json
```

This checks sorting against selection on 2,048 seeded windows of 9, 25, 49, and 81
samples, reuses one scratch vector for both algorithms, and measures 200 passes
per repetition after three warmups. It isolates the choice of median algorithm; it
does not measure whole-image filtering.

### Aggregate loading and OCR jobs

The aggregate-loader benchmark uses published JPEG pages and four sequential
passes, comparing ordinary decoding with the bounded cache. The OCR job benchmark
uses an injected engine and temporary model packs; its timings cover verification
and cache bookkeeping, not real inference:

```sh
cargo test --release --no-default-features --features ocrs,onnx --locked --lib \
  release_batch_aggregate_cache_benchmark_json \
  -- --ignored --nocapture --test-threads=1
cargo test --release --no-default-features --features ocrs,onnx --locked --lib \
  release_ocr_job_cache_benchmark_json \
  -- --ignored --nocapture --test-threads=1
```

Both emit JSON with three warmups and ten measured repetitions. Aggregate results
include actual decoder calls, and OCR results include observed model-file reads
and engine constructions. Use these counts as deterministic regression evidence
independently of timing.
