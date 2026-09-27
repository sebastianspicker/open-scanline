//! Film profiles (OSL-FILM): catalog, inversion, orange-mask removal.

use crate::domain::image::{ImageBuffer, PixelFormat};
use crate::error::{Result, ScanError};
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

#[derive(Clone, Copy)]
enum FilmKind {
    ColorNegative,
    Slide,
    BwNegative,
}

impl FilmKind {
    fn metadata(self) -> (&'static str, &'static str) {
        match self {
            Self::ColorNegative => ("color_negative", "color negative profile."),
            Self::Slide => ("slide", "slide/positive profile."),
            Self::BwNegative => ("bw_negative", "black-and-white negative profile."),
        }
    }
}

struct FilmProfileData(
    &'static str,
    &'static str,
    FilmKind,
    &'static str,
    Option<(f64, f64, f64)>,
    f64,
    f64,
);

impl FilmProfileData {
    fn to_profile(&self) -> FilmProfile {
        let (kind, note_suffix) = self.2.metadata();
        FilmProfile {
            id: self.0.into(),
            name: self.1.into(),
            kind: kind.into(),
            manufacturer: self.3.into(),
            note: format!("{} {note_suffix}", self.1),
            orange_mask: self.4,
            contrast: self.5,
            gamma: self.6,
        }
    }
}

#[rustfmt::skip]
const FILM_CATALOG: &[FilmProfileData] = &[
    FilmProfileData("generic_color_negative", "Generic Color Negative", FilmKind::ColorNegative, "Generic", Some((1.15, 0.62, 0.45)), 1.15, 1.0),
    FilmProfileData("kodak_gold_200", "Kodak Gold 200", FilmKind::ColorNegative, "Kodak", Some((1.18, 0.60, 0.42)), 1.12, 1.05),
    FilmProfileData("kodak_gold_100", "Kodak Gold 100", FilmKind::ColorNegative, "Kodak", Some((1.16, 0.61, 0.44)), 1.10, 1.04),
    FilmProfileData("kodak_gold_400", "Kodak Gold 400", FilmKind::ColorNegative, "Kodak", Some((1.17, 0.59, 0.43)), 1.14, 1.05),
    FilmProfileData("kodak_portra_160", "Kodak Portra 160", FilmKind::ColorNegative, "Kodak", Some((1.11, 0.62, 0.47)), 1.08, 1.08),
    FilmProfileData("kodak_portra_400", "Kodak Portra 400", FilmKind::ColorNegative, "Kodak", Some((1.12, 0.61, 0.46)), 1.10, 1.08),
    FilmProfileData("kodak_portra_800", "Kodak Portra 800", FilmKind::ColorNegative, "Kodak", Some((1.13, 0.60, 0.45)), 1.12, 1.06),
    FilmProfileData("kodak_ektar_100", "Kodak Ektar 100", FilmKind::ColorNegative, "Kodak", Some((1.20, 0.60, 0.40)), 1.20, 1.02),
    FilmProfileData("kodak_colorplus_200", "Kodak ColorPlus 200", FilmKind::ColorNegative, "Kodak", Some((1.17, 0.59, 0.44)), 1.14, 1.04),
    FilmProfileData("kodak_ultramax_400", "Kodak UltraMax 400", FilmKind::ColorNegative, "Kodak", Some((1.16, 0.58, 0.43)), 1.13, 1.06),
    FilmProfileData("fuji_superia_100", "Fuji Superia 100", FilmKind::ColorNegative, "Fuji", Some((1.09, 0.64, 0.48)), 1.10, 1.05),
    FilmProfileData("fuji_superia_200", "Fuji Superia 200", FilmKind::ColorNegative, "Fuji", Some((1.10, 0.63, 0.47)), 1.11, 1.05),
    FilmProfileData("fuji_superia_400", "Fuji Superia 400", FilmKind::ColorNegative, "Fuji", Some((1.10, 0.63, 0.47)), 1.12, 1.05),
    FilmProfileData("fuji_pro_400h", "Fuji Pro 400H", FilmKind::ColorNegative, "Fuji", Some((1.08, 0.62, 0.48)), 1.08, 1.10),
    FilmProfileData("agfa_vista_200", "Agfa Vista 200", FilmKind::ColorNegative, "Agfa", Some((1.14, 0.61, 0.45)), 1.16, 1.03),
    FilmProfileData("cinestill_800t", "CineStill 800T", FilmKind::ColorNegative, "CineStill", Some((1.05, 0.58, 0.70)), 1.18, 1.0),
    FilmProfileData("lomography_color_400", "Lomography Color 400", FilmKind::ColorNegative, "Lomography", Some((1.17, 0.57, 0.41)), 1.20, 1.0),
    FilmProfileData("konica_centuria_200", "Konica Centuria 200", FilmKind::ColorNegative, "Konica", Some((1.14, 0.60, 0.44)), 1.13, 1.04),
    FilmProfileData("generic_consumer_cn", "Generic Consumer Color Negative", FilmKind::ColorNegative, "Generic", Some((1.16, 0.60, 0.44)), 1.14, 1.04),
    FilmProfileData("generic_portrait_cn", "Generic Portrait Color Negative", FilmKind::ColorNegative, "Generic", Some((1.11, 0.62, 0.47)), 1.09, 1.08),
    FilmProfileData("generic_slide", "Generic Slide", FilmKind::Slide, "Generic", None, 1.0, 1.0),
    FilmProfileData("fuji_velvia_50", "Fuji Velvia 50", FilmKind::Slide, "Fuji", None, 1.15, 0.95),
    FilmProfileData("fuji_velvia_100", "Fuji Velvia 100", FilmKind::Slide, "Fuji", None, 1.12, 0.97),
    FilmProfileData("fuji_provia_100f", "Fuji Provia 100F", FilmKind::Slide, "Fuji", None, 1.0, 1.0),
    FilmProfileData("kodak_ektachrome_e100", "Kodak Ektachrome E100", FilmKind::Slide, "Kodak", None, 1.05, 1.05),
    FilmProfileData("kodachrome_64", "Kodachrome 64", FilmKind::Slide, "Kodak", None, 1.08, 1.0),
    FilmProfileData("agfa_ct_precisa_100", "Agfa CT Precisa 100", FilmKind::Slide, "Agfa", None, 1.06, 1.02),
    FilmProfileData("generic_e6_slide", "Generic E-6 Slide", FilmKind::Slide, "Generic", None, 1.04, 1.02),
    FilmProfileData("generic_bw_negative", "Generic B&W Negative", FilmKind::BwNegative, "Generic", None, 1.2, 1.0),
    FilmProfileData("ilford_hp5_plus", "Ilford HP5 Plus", FilmKind::BwNegative, "Ilford", None, 1.25, 1.0),
    FilmProfileData("ilford_fp4_plus", "Ilford FP4 Plus", FilmKind::BwNegative, "Ilford", None, 1.15, 1.05),
    FilmProfileData("ilford_delta_100", "Ilford Delta 100", FilmKind::BwNegative, "Ilford", None, 1.28, 0.96),
    FilmProfileData("ilford_delta_400", "Ilford Delta 400", FilmKind::BwNegative, "Ilford", None, 1.30, 0.95),
    FilmProfileData("kodak_tri_x_400", "Kodak Tri-X 400", FilmKind::BwNegative, "Kodak", None, 1.28, 1.0),
    FilmProfileData("kodak_tmax_100", "Kodak T-Max 100", FilmKind::BwNegative, "Kodak", None, 1.30, 0.98),
    FilmProfileData("kodak_tmax_400", "Kodak T-Max 400", FilmKind::BwNegative, "Kodak", None, 1.35, 0.92),
    FilmProfileData("fuji_acros_100", "Fuji Acros 100", FilmKind::BwNegative, "Fuji", None, 1.24, 1.0),
    FilmProfileData("fomapan_100", "Fomapan 100", FilmKind::BwNegative, "Foma", None, 1.20, 1.02),
    FilmProfileData("fomapan_400", "Fomapan 400", FilmKind::BwNegative, "Foma", None, 1.24, 1.0),
    FilmProfileData("rolle_rp_x_400", "Rollei RPX 400", FilmKind::BwNegative, "Rollei", None, 1.26, 1.0),
    FilmProfileData("kodak_ultramax_800", "Kodak UltraMax 800", FilmKind::ColorNegative, "Kodak", Some((1.15, 0.57, 0.42)), 1.16, 1.04),
    FilmProfileData("fuji_superia_800", "Fuji Superia 800", FilmKind::ColorNegative, "Fuji", Some((1.11, 0.62, 0.46)), 1.14, 1.04),
    FilmProfileData("fuji_c200", "Fuji C200", FilmKind::ColorNegative, "Fuji", Some((1.09, 0.64, 0.48)), 1.10, 1.06),
    FilmProfileData("agfa_vista_100", "Agfa Vista 100", FilmKind::ColorNegative, "Agfa", Some((1.14, 0.61, 0.45)), 1.14, 1.03),
    FilmProfileData("agfa_vista_400", "Agfa Vista 400", FilmKind::ColorNegative, "Agfa", Some((1.15, 0.60, 0.44)), 1.17, 1.02),
    FilmProfileData("cinestill_50d", "CineStill 50D", FilmKind::ColorNegative, "CineStill", Some((1.14, 0.61, 0.44)), 1.12, 1.04),
    FilmProfileData("fuji_astia_100f", "Fuji Astia 100F", FilmKind::Slide, "Fuji", None, 1.02, 1.02),
    FilmProfileData("kodachrome_25", "Kodachrome 25", FilmKind::Slide, "Kodak", None, 1.10, 0.98),
    FilmProfileData("ilford_pan_f_plus", "Ilford Pan F Plus", FilmKind::BwNegative, "Ilford", None, 1.22, 1.02),
    FilmProfileData("ilford_xp2_super", "Ilford XP2 Super", FilmKind::BwNegative, "Ilford", None, 1.18, 1.04),
    FilmProfileData("fuji_neopan_400", "Fuji Neopan 400", FilmKind::BwNegative, "Fuji", None, 1.22, 1.03),
];

/// Built-in film catalog (≥50 profiles, matches open-scanline productive set).
pub fn list_film_profiles() -> Vec<FilmProfile> {
    FILM_CATALOG
        .iter()
        .map(FilmProfileData::to_profile)
        .collect()
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
