//! Film profiles (OSL-FILM): catalog, inversion, orange-mask removal.

use crate::core::{ImageBuffer, PixelFormat, Result, ScanError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilmProfile {
    pub id: String,
    pub name: String,
    /// "color_negative" | "bw_negative" | "slide"
    pub kind: String,
    pub manufacturer: String,
    pub note: String,
    pub orange_mask: Option<(f64, f64, f64)>,
    pub contrast: f64,
    pub gamma: f64,
}

fn cn(
    id: &str,
    name: &str,
    mfr: &str,
    mask: (f64, f64, f64),
    contrast: f64,
    gamma: f64,
) -> FilmProfile {
    FilmProfile {
        id: id.into(),
        name: name.into(),
        kind: "color_negative".into(),
        manufacturer: mfr.into(),
        note: format!("{name} color negative profile."),
        orange_mask: Some(mask),
        contrast,
        gamma,
    }
}

fn sl(id: &str, name: &str, mfr: &str, contrast: f64, gamma: f64) -> FilmProfile {
    FilmProfile {
        id: id.into(),
        name: name.into(),
        kind: "slide".into(),
        manufacturer: mfr.into(),
        note: format!("{name} slide/positive profile."),
        orange_mask: None,
        contrast,
        gamma,
    }
}

fn bw(id: &str, name: &str, mfr: &str, contrast: f64, gamma: f64) -> FilmProfile {
    FilmProfile {
        id: id.into(),
        name: name.into(),
        kind: "bw_negative".into(),
        manufacturer: mfr.into(),
        note: format!("{name} black-and-white negative profile."),
        orange_mask: None,
        contrast,
        gamma,
    }
}

/// Built-in film catalog (≥50 profiles, matches open-scanline productive set).
pub fn list_film_profiles() -> Vec<FilmProfile> {
    vec![
        cn(
            "generic_color_negative",
            "Generic Color Negative",
            "Generic",
            (1.15, 0.62, 0.45),
            1.15,
            1.0,
        ),
        cn(
            "kodak_gold_200",
            "Kodak Gold 200",
            "Kodak",
            (1.18, 0.60, 0.42),
            1.12,
            1.05,
        ),
        cn(
            "kodak_gold_100",
            "Kodak Gold 100",
            "Kodak",
            (1.16, 0.61, 0.44),
            1.10,
            1.04,
        ),
        cn(
            "kodak_gold_400",
            "Kodak Gold 400",
            "Kodak",
            (1.17, 0.59, 0.43),
            1.14,
            1.05,
        ),
        cn(
            "kodak_portra_160",
            "Kodak Portra 160",
            "Kodak",
            (1.11, 0.62, 0.47),
            1.08,
            1.08,
        ),
        cn(
            "kodak_portra_400",
            "Kodak Portra 400",
            "Kodak",
            (1.12, 0.61, 0.46),
            1.10,
            1.08,
        ),
        cn(
            "kodak_portra_800",
            "Kodak Portra 800",
            "Kodak",
            (1.13, 0.60, 0.45),
            1.12,
            1.06,
        ),
        cn(
            "kodak_ektar_100",
            "Kodak Ektar 100",
            "Kodak",
            (1.20, 0.60, 0.40),
            1.20,
            1.02,
        ),
        cn(
            "kodak_colorplus_200",
            "Kodak ColorPlus 200",
            "Kodak",
            (1.17, 0.59, 0.44),
            1.14,
            1.04,
        ),
        cn(
            "kodak_ultramax_400",
            "Kodak UltraMax 400",
            "Kodak",
            (1.16, 0.58, 0.43),
            1.13,
            1.06,
        ),
        cn(
            "fuji_superia_100",
            "Fuji Superia 100",
            "Fuji",
            (1.09, 0.64, 0.48),
            1.10,
            1.05,
        ),
        cn(
            "fuji_superia_200",
            "Fuji Superia 200",
            "Fuji",
            (1.10, 0.63, 0.47),
            1.11,
            1.05,
        ),
        cn(
            "fuji_superia_400",
            "Fuji Superia 400",
            "Fuji",
            (1.10, 0.63, 0.47),
            1.12,
            1.05,
        ),
        cn(
            "fuji_pro_400h",
            "Fuji Pro 400H",
            "Fuji",
            (1.08, 0.62, 0.48),
            1.08,
            1.10,
        ),
        cn(
            "agfa_vista_200",
            "Agfa Vista 200",
            "Agfa",
            (1.14, 0.61, 0.45),
            1.16,
            1.03,
        ),
        cn(
            "cinestill_800t",
            "CineStill 800T",
            "CineStill",
            (1.05, 0.58, 0.70),
            1.18,
            1.0,
        ),
        cn(
            "lomography_color_400",
            "Lomography Color 400",
            "Lomography",
            (1.17, 0.57, 0.41),
            1.20,
            1.0,
        ),
        cn(
            "konica_centuria_200",
            "Konica Centuria 200",
            "Konica",
            (1.14, 0.60, 0.44),
            1.13,
            1.04,
        ),
        cn(
            "generic_consumer_cn",
            "Generic Consumer Color Negative",
            "Generic",
            (1.16, 0.60, 0.44),
            1.14,
            1.04,
        ),
        cn(
            "generic_portrait_cn",
            "Generic Portrait Color Negative",
            "Generic",
            (1.11, 0.62, 0.47),
            1.09,
            1.08,
        ),
        sl("generic_slide", "Generic Slide", "Generic", 1.0, 1.0),
        sl("fuji_velvia_50", "Fuji Velvia 50", "Fuji", 1.15, 0.95),
        sl("fuji_velvia_100", "Fuji Velvia 100", "Fuji", 1.12, 0.97),
        sl("fuji_provia_100f", "Fuji Provia 100F", "Fuji", 1.0, 1.0),
        sl(
            "kodak_ektachrome_e100",
            "Kodak Ektachrome E100",
            "Kodak",
            1.05,
            1.05,
        ),
        sl("kodachrome_64", "Kodachrome 64", "Kodak", 1.08, 1.0),
        sl(
            "agfa_ct_precisa_100",
            "Agfa CT Precisa 100",
            "Agfa",
            1.06,
            1.02,
        ),
        sl(
            "generic_e6_slide",
            "Generic E-6 Slide",
            "Generic",
            1.04,
            1.02,
        ),
        bw(
            "generic_bw_negative",
            "Generic B&W Negative",
            "Generic",
            1.2,
            1.0,
        ),
        bw("ilford_hp5_plus", "Ilford HP5 Plus", "Ilford", 1.25, 1.0),
        bw("ilford_fp4_plus", "Ilford FP4 Plus", "Ilford", 1.15, 1.05),
        bw("ilford_delta_100", "Ilford Delta 100", "Ilford", 1.28, 0.96),
        bw("ilford_delta_400", "Ilford Delta 400", "Ilford", 1.30, 0.95),
        bw("kodak_tri_x_400", "Kodak Tri-X 400", "Kodak", 1.28, 1.0),
        bw("kodak_tmax_100", "Kodak T-Max 100", "Kodak", 1.30, 0.98),
        bw("kodak_tmax_400", "Kodak T-Max 400", "Kodak", 1.35, 0.92),
        bw("fuji_acros_100", "Fuji Acros 100", "Fuji", 1.24, 1.0),
        bw("fomapan_100", "Fomapan 100", "Foma", 1.20, 1.02),
        bw("fomapan_400", "Fomapan 400", "Foma", 1.24, 1.0),
        bw("rolle_rp_x_400", "Rollei RPX 400", "Rollei", 1.26, 1.0),
        // Extra entries for ≥50 catalog
        cn(
            "kodak_ultramax_800",
            "Kodak UltraMax 800",
            "Kodak",
            (1.15, 0.57, 0.42),
            1.16,
            1.04,
        ),
        cn(
            "fuji_superia_800",
            "Fuji Superia 800",
            "Fuji",
            (1.11, 0.62, 0.46),
            1.14,
            1.04,
        ),
        cn(
            "fuji_c200",
            "Fuji C200",
            "Fuji",
            (1.09, 0.64, 0.48),
            1.10,
            1.06,
        ),
        cn(
            "agfa_vista_100",
            "Agfa Vista 100",
            "Agfa",
            (1.14, 0.61, 0.45),
            1.14,
            1.03,
        ),
        cn(
            "agfa_vista_400",
            "Agfa Vista 400",
            "Agfa",
            (1.15, 0.60, 0.44),
            1.17,
            1.02,
        ),
        cn(
            "cinestill_50d",
            "CineStill 50D",
            "CineStill",
            (1.14, 0.61, 0.44),
            1.12,
            1.04,
        ),
        sl("fuji_astia_100f", "Fuji Astia 100F", "Fuji", 1.02, 1.02),
        sl("kodachrome_25", "Kodachrome 25", "Kodak", 1.10, 0.98),
        bw(
            "ilford_pan_f_plus",
            "Ilford Pan F Plus",
            "Ilford",
            1.22,
            1.02,
        ),
        bw("ilford_xp2_super", "Ilford XP2 Super", "Ilford", 1.18, 1.04),
        bw("fuji_neopan_400", "Fuji Neopan 400", "Fuji", 1.22, 1.03),
    ]
}

pub fn get_film_profile(id: &str) -> Result<FilmProfile> {
    list_film_profiles()
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| ScanError::Invalid(format!("film profile not found: {id}")))
}

fn clamp_u8(v: f64) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

/// Convert film scan buffer using profile (invert + orange mask + contrast/gamma).
pub fn convert_film(image: &ImageBuffer, profile_id: &str) -> Result<ImageBuffer> {
    let profile = get_film_profile(profile_id)?;
    let bpp = image.bpp();
    let mut out = image.data.clone();

    invert_negative(&mut out, bpp, &profile.kind);
    remove_orange_mask(&mut out, bpp, image.pixel_format, profile.orange_mask);
    apply_tone_curve(&mut out, bpp, profile.contrast, profile.gamma);
    desaturate_bw(&mut out, bpp, &profile.kind);

    ImageBuffer::new(image.width, image.height, image.pixel_format, out)
}

fn invert_negative(data: &mut [u8], bpp: usize, kind: &str) {
    if kind.contains("negative") {
        for pixel in data.chunks_exact_mut(bpp) {
            for channel in &mut pixel[..bpp.min(3)] {
                *channel = 255 - *channel;
            }
        }
    }
}

fn remove_orange_mask(
    data: &mut [u8],
    bpp: usize,
    pixel_format: PixelFormat,
    orange_mask: Option<(f64, f64, f64)>,
) {
    let Some(mask) = orange_mask else {
        return;
    };
    if pixel_format != PixelFormat::Rgb8 && pixel_format != PixelFormat::Rgba8 {
        return;
    }

    let (red_gain, green_gain, blue_gain) = orange_mask_gains(mask);
    for pixel in data.chunks_exact_mut(bpp) {
        pixel[0] = clamp_u8(pixel[0] as f64 * red_gain);
        pixel[1] = clamp_u8(pixel[1] as f64 * green_gain);
        pixel[2] = clamp_u8(pixel[2] as f64 * blue_gain);
    }
}

fn orange_mask_gains((red, green, blue): (f64, f64, f64)) -> (f64, f64, f64) {
    let mean = (red + green + blue) / 3.0;
    (
        if red > 1e-6 { mean / red } else { 1.0 },
        if green > 1e-6 { mean / green } else { 1.0 },
        if blue > 1e-6 { mean / blue } else { 1.0 },
    )
}

fn apply_tone_curve(data: &mut [u8], bpp: usize, contrast: f64, gamma: f64) {
    let gamma = if gamma <= 0.0 { 1.0 } else { gamma };
    for pixel in data.chunks_exact_mut(bpp) {
        for channel in &mut pixel[..bpp.min(3)] {
            let tone = (*channel as f64 / 255.0 - 0.5) * contrast + 0.5;
            *channel = clamp_u8(tone.clamp(0.0, 1.0).powf(1.0 / gamma) * 255.0);
        }
    }
}

fn desaturate_bw(data: &mut [u8], bpp: usize, kind: &str) {
    if !kind.starts_with("bw") || bpp < 3 {
        return;
    }

    for pixel in data.chunks_exact_mut(bpp) {
        let luma =
            ((77u32 * pixel[0] as u32 + 150 * pixel[1] as u32 + 29 * pixel[2] as u32) >> 8) as u8;
        pixel[0] = luma;
        pixel[1] = luma;
        pixel[2] = luma;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_at_least_50() {
        let list = list_film_profiles();
        assert!(list.len() >= 50, "got {}", list.len());
        assert!(get_film_profile("kodak_portra_400").is_ok());
    }

    #[test]
    fn catalog_order_and_representative_data_are_stable() {
        let catalog = list_film_profiles();
        assert_eq!(catalog[0].id, "generic_color_negative");
        assert_eq!(catalog[0].orange_mask, Some((1.15, 0.62, 0.45)));
        assert_eq!(catalog[20].id, "generic_slide");
        assert_eq!(catalog[20].kind, "slide");
        assert_eq!(catalog[28].id, "generic_bw_negative");
        assert_eq!(catalog[28].kind, "bw_negative");
        assert_eq!(catalog.last().unwrap().id, "fuji_neopan_400");
    }

    #[test]
    fn color_negative_characterizes_rgb_and_rgba_processing() {
        let rgb = ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![200, 100, 50]).unwrap();
        let rgba = ImageBuffer::new(1, 1, PixelFormat::Rgba8, vec![200, 100, 50, 100]).unwrap();

        assert_eq!(
            convert_film(&rgb, "generic_color_negative").unwrap().data,
            vec![21, 194, 255]
        );
        assert_eq!(
            convert_film(&rgba, "generic_color_negative").unwrap().data,
            vec![21, 194, 255, 100]
        );
    }

    #[test]
    fn slide_characterizes_rgb_rgba_and_gray_processing() {
        let rgb = ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![0, 128, 255]).unwrap();
        let rgba = ImageBuffer::new(1, 1, PixelFormat::Rgba8, vec![12, 64, 192, 33]).unwrap();
        let gray = ImageBuffer::new(2, 1, PixelFormat::Gray8, vec![10, 200]).unwrap();

        assert_eq!(
            convert_film(&rgb, "generic_slide").unwrap().data,
            vec![0, 128, 255]
        );
        assert_eq!(
            convert_film(&rgba, "generic_slide").unwrap().data,
            vec![12, 64, 192, 33]
        );
        assert_eq!(
            convert_film(&gray, "generic_slide").unwrap().data,
            vec![10, 200]
        );
    }

    #[test]
    fn bw_negative_characterizes_rgb_rgba_and_gray_processing() {
        let rgb = ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![200, 100, 50]).unwrap();
        let rgba = ImageBuffer::new(1, 1, PixelFormat::Rgba8, vec![200, 100, 50, 17]).unwrap();
        let gray = ImageBuffer::new(2, 1, PixelFormat::Gray8, vec![200, 100]).unwrap();

        assert_eq!(
            convert_film(&rgb, "generic_bw_negative").unwrap().data,
            vec![131, 131, 131]
        );
        assert_eq!(
            convert_film(&rgba, "generic_bw_negative").unwrap().data,
            vec![131, 131, 131, 17]
        );
        assert_eq!(
            convert_film(&gray, "generic_bw_negative").unwrap().data,
            vec![41, 161]
        );
    }

    #[test]
    fn every_film_profile_preserves_rgba_alpha() {
        for profile in list_film_profiles() {
            let image = ImageBuffer::new(
                2,
                1,
                PixelFormat::Rgba8,
                vec![200, 100, 50, 17, 12, 64, 192, 231],
            )
            .unwrap();
            let output = convert_film(&image, &profile.id).unwrap();
            assert_eq!(
                output
                    .data
                    .chunks_exact(4)
                    .map(|pixel| pixel[3])
                    .collect::<Vec<_>>(),
                vec![17, 231],
                "{} changed alpha",
                profile.id
            );
        }
    }
}
