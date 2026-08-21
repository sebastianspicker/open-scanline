use open_scanline::packaging::{build_portable, PackagingOptions};

#[test]
fn hostile_binary_name_is_rejected_before_output_creation() {
    let root = std::env::temp_dir().join(format!(
        "open_scanline_portable_contract_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let binary = root.join("scan;injected");
    std::fs::write(&binary, b"not executed").unwrap();
    let out = root.join("not-created").join("portable.zip");

    let error = build_portable(&PackagingOptions {
        binary,
        out: out.clone(),
    })
    .unwrap_err();
    assert!(error.to_string().contains("ASCII letters"));
    assert!(!out.parent().unwrap().exists());
    let _ = std::fs::remove_dir_all(root);
}
