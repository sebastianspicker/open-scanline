use super::clamp_f;

pub(super) fn restore_rgb(data: &[u8], bpp: usize, pixels: usize) -> Vec<u8> {
    let clip = (pixels as f64 * 0.005).max(1.0) as u32;
    let histograms = rgb_histograms(data, bpp);
    let luts = histograms.map(|histogram| stretch_lut(histogram_span(&histogram, clip)));
    let (stretched, sums) = stretch_rgb(data, bpp, &luts);
    gray_world_gains(sums, pixels).map_or(stretched.clone(), |gains| {
        apply_rgb_gains(&stretched, bpp, gains)
    })
}

fn rgb_histograms(data: &[u8], bpp: usize) -> [[u32; 256]; 3] {
    let mut histograms = [[0u32; 256]; 3];
    for pixel in data.chunks_exact(bpp) {
        for channel in 0..3 {
            histograms[channel][pixel[channel] as usize] += 1;
        }
    }
    histograms
}

fn histogram_span(histogram: &[u32; 256], clip: u32) -> (i32, i32) {
    let lo = histogram
        .iter()
        .scan(0u32, |sum, &count| {
            *sum += count;
            Some(*sum)
        })
        .position(|sum| sum > clip)
        .unwrap_or(0) as i32;
    let mut hi = histogram
        .iter()
        .rev()
        .scan(0u32, |sum, &count| {
            *sum += count;
            Some(*sum)
        })
        .position(|sum| sum > clip)
        .map_or(255, |offset| 255 - offset as i32);
    if hi <= lo {
        hi = (lo + 1).min(255);
    }
    (lo, hi)
}

fn stretch_lut((lo, hi): (i32, i32)) -> [u8; 256] {
    let mut lut = [0u8; 256];
    for (value, item) in lut.iter_mut().enumerate() {
        *item = clamp_f(((value as i32 - lo) as f64 / (hi - lo) as f64).clamp(0.0, 1.0) * 255.0);
    }
    lut
}

fn stretch_rgb(data: &[u8], bpp: usize, luts: &[[u8; 256]; 3]) -> (Vec<u8>, [f64; 3]) {
    let mut stretched = data.to_vec();
    let mut sums = [0.0; 3];
    for pixel in stretched.chunks_exact_mut(bpp) {
        for channel in 0..3 {
            pixel[channel] = luts[channel][pixel[channel] as usize];
            sums[channel] += pixel[channel] as f64;
        }
    }
    (stretched, sums)
}

fn gray_world_gains(sums: [f64; 3], pixels: usize) -> Option<[f64; 3]> {
    let means = sums.map(|sum| sum / pixels as f64);
    if means.iter().any(|&mean| mean < 1e-6) {
        return None;
    }
    let mean = means.iter().sum::<f64>() / 3.0;
    Some(means.map(|channel| 1.0 + (mean / channel - 1.0) * 0.85))
}

fn apply_rgb_gains(data: &[u8], bpp: usize, gains: [f64; 3]) -> Vec<u8> {
    let mut out = data.to_vec();
    for pixel in out.chunks_exact_mut(bpp) {
        for channel in 0..3 {
            pixel[channel] = clamp_f(pixel[channel] as f64 * gains[channel]);
        }
    }
    out
}
