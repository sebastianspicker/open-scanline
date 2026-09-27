//! Isolate neighborhood selection cost with seeded, equally reused scratch buffers.
use serde_json::{json, Value};
use std::hint::black_box;
use std::time::Instant;

const SEED: u32 = 0x51a7_0ff5;
const WINDOWS: usize = 2048;
const PASSES: usize = 200;

fn main() {
    let mut results = Vec::new();
    for length in [9, 25, 49, 81] {
        let windows = windows(length);
        validate(&windows);
        results.push(measure(&windows, false));
        results.push(measure(&windows, true));
    }
    println!(
        "{}",
        json!({"seed":SEED,"warmups":3,"repetitions":10,
        "windows":WINDOWS,"passes":PASSES,"profile":"release","results":results})
    );
}

fn windows(length: usize) -> Vec<Vec<u8>> {
    let mut state = SEED;
    (0..WINDOWS)
        .map(|_| {
            (0..length)
                .map(|_| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (state >> 24) as u8
                })
                .collect()
        })
        .collect()
}

fn median(values: &mut [u8], select: bool) -> u8 {
    let middle = values.len() / 2;
    if select {
        *values.select_nth_unstable(middle).1
    } else {
        values.sort_unstable();
        values[middle]
    }
}

fn validate(windows: &[Vec<u8>]) {
    for window in windows {
        assert_eq!(
            median(&mut window.clone(), false),
            median(&mut window.clone(), true)
        );
    }
}

fn run(windows: &[Vec<u8>], select: bool) {
    let mut scratch = vec![0; windows[0].len()];
    let mut checksum = 0_u64;
    for _ in 0..PASSES {
        for window in windows {
            scratch.copy_from_slice(window);
            checksum += u64::from(median(&mut scratch, select));
        }
    }
    black_box(checksum);
}

fn measure(windows: &[Vec<u8>], select: bool) -> Value {
    for _ in 0..3 {
        run(windows, select);
    }
    let samples: Vec<_> = (0..10)
        .map(|_| {
            let start = Instant::now();
            run(windows, select);
            start.elapsed().as_nanos() as u64
        })
        .collect();
    json!({"length":windows[0].len(),"algorithm":if select {"select"} else {"sort"},
        "samples_ns":samples})
}
