use std::path::Path;

pub(super) fn parse_crop(s: Option<&str>) -> Option<[i32; 4]> {
    let s = s?;
    match crate::domain::image::parse_crop_components(s) {
        Ok(crop) => Some(crop),
        Err(crate::domain::image::CropParseError::ComponentCount) => {
            eprintln!("--crop must be x,y,w,h");
            std::process::exit(2);
        }
        Err(crate::domain::image::CropParseError::InvalidInteger(_)) => {
            eprintln!("--crop must be x,y,w,h integers");
            std::process::exit(2);
        }
    }
}

pub(super) fn print_wrote(path: &Path) {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    println!("wrote {} ({size} bytes)", path.display());
}
