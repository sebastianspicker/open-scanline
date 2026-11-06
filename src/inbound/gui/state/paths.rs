use super::*;

impl GuiState {
    /// First scan index past every existing `{name}_scan_NNN.{ext}` in the
    /// output folder, so a restart does not replace earlier scans.
    pub(in crate::inbound::gui) fn first_free_frame_index(&self) -> i32 {
        let Ok(output_name) = validate_output_name(&self.output_name) else {
            return 0;
        };
        let ext = normalize_image_ext(&self.output_fmt, "png");
        std::fs::read_dir(&self.output_dir).map_or(0, |entries| {
            next_frame_index(
                entries.filter_map(|entry| entry.ok()?.file_name().into_string().ok()),
                output_name,
                &ext,
            )
        })
    }

    pub(in crate::inbound::gui) fn out_path(&self, suffix: &str) -> Result<PathBuf> {
        let output_name = validate_output_name(&self.output_name)?;
        let ext = normalize_image_ext(&self.output_fmt, "png");
        Ok(PathBuf::from(&self.output_dir).join(format!(
            "{output_name}_{suffix}_{:03}.{ext}",
            self.frame_index
        )))
    }

    pub(in crate::inbound::gui) fn out_path_plain(&self, suffix: &str) -> Result<PathBuf> {
        let output_name = validate_output_name(&self.output_name)?;
        let ext = normalize_image_ext(&self.output_fmt, "png");
        Ok(PathBuf::from(&self.output_dir).join(format!("{output_name}_{suffix}.{ext}")))
    }
}

/// One past the highest `{output_name}_scan_NNN.{ext}` index among `names`; 0 if none.
fn next_frame_index(names: impl Iterator<Item = String>, output_name: &str, ext: &str) -> i32 {
    let prefix = format!("{output_name}_scan_");
    let suffix = format!(".{ext}");
    names
        .filter_map(|name| {
            let digits = name.strip_prefix(&prefix)?.strip_suffix(&suffix)?;
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            digits.parse::<i32>().ok()
        })
        .max()
        .map_or(0, |index| index.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_frame_index_skips_past_the_highest_matching_name() {
        let next = |list: &[&str]| {
            next_frame_index(list.iter().map(|name| name.to_string()), "scan", "png")
        };
        assert_eq!(next(&[]), 0);
        assert_eq!(
            next(&[
                "scan_scan_000.png",
                "scan_scan_007.png",
                "scan_scan_003.png",
                "scan_scan_9x.png",
                "scan_scan_012.tif",
                "other_scan_050.png",
                "scan_raw_020.png",
                "scan_scan_.png",
            ]),
            8
        );
        assert_eq!(next(&["scan_scan_1000.png"]), 1001);
    }

    #[test]
    fn startup_frame_index_reads_the_output_folder() {
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "open-scanline-frames-{}-{}",
            std::process::id(),
            u128::from_le_bytes(nonce)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("scan_scan_004.png"), b"").unwrap();
        let mut state = GuiState::new(None);
        state.output_name = "scan".into();
        state.output_fmt = "png".into();
        state.output_dir = dir.display().to_string();
        assert_eq!(state.first_free_frame_index(), 5);
        state.output_dir = dir.join("missing").display().to_string();
        assert_eq!(state.first_free_frame_index(), 0);
        // Only the file and folder this test created, never recursively.
        std::fs::remove_file(dir.join("scan_scan_004.png")).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
}
