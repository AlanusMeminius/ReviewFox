//! Material Icon Theme (MIT) file-type icons for tree rows.
//!
//! Associations and SVGs are vendored by `tools/gen_material_icons.mjs` from the
//! official `material-icon-theme` package (see `file_icons_generated.rs`). Lookup
//! mirrors VS Code: exact file name first, then the longest dotted extension
//! suffix (`a.spec.ts` tries `spec.ts`, then `ts`), then the default file icon.
//! Folders keep the Lucide icons; language-id associations are not used.

use super::file_icons_generated::{DEFAULT_FILE_ICON, FILE_EXTENSIONS, FILE_NAMES, MATERIAL_ICONS};

fn find(table: &'static [(&str, &str)], key: &str) -> Option<&'static str> {
    table
        .binary_search_by(|(k, _)| (*k).cmp(key))
        .ok()
        .map(|i| table[i].1)
}

/// Asset path (e.g. `material/rust.svg`) for a repo-relative or bare file path.
pub(super) fn file_icon_for_path(path: &str) -> &'static str {
    let base = path.rsplit('/').next().unwrap_or(path);
    let name = base.to_lowercase();
    if let Some(icon) = find(FILE_NAMES, &name) {
        return icon;
    }
    // A leading dot is not an extension separator, but Material lists a few
    // dot-prefixed keys (`.ncurc.js`) that only match the whole name.
    if let Some(icon) = name
        .starts_with('.')
        .then(|| find(FILE_EXTENSIONS, &name))
        .flatten()
    {
        return icon;
    }
    for (i, _) in name.match_indices('.') {
        if let Some(icon) = find(FILE_EXTENSIONS, &name[i + 1..]) {
            return icon;
        }
    }
    DEFAULT_FILE_ICON
}

/// Bytes for an embedded `material/*.svg` asset.
pub(super) fn material_asset(path: &str) -> Option<&'static [u8]> {
    MATERIAL_ICONS
        .binary_search_by(|(k, _)| (*k).cmp(path))
        .ok()
        .map(|i| MATERIAL_ICONS[i].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_resolves() {
        assert_eq!(file_icon_for_path("src/ui/mod.rs"), "material/rust.svg");
        assert_eq!(file_icon_for_path("Main.RS"), "material/rust.svg");
    }

    #[test]
    fn file_name_beats_extension() {
        assert_eq!(file_icon_for_path("CMakeLists.txt"), "material/cmake.svg");
        assert_eq!(
            file_icon_for_path("a/b/CMakeLists.txt"),
            "material/cmake.svg"
        );
        assert_ne!(file_icon_for_path("notes.txt"), "material/cmake.svg");
    }

    #[test]
    fn longest_extension_suffix_wins() {
        assert_eq!(file_icon_for_path("foo.d.ts"), file_icon_for_path("x.d.ts"));
        assert_ne!(file_icon_for_path("foo.d.ts"), file_icon_for_path("foo.ts"));
    }

    #[test]
    fn unknown_falls_back_to_default() {
        assert_eq!(file_icon_for_path("weird.zzzunknown"), DEFAULT_FILE_ICON);
        assert_eq!(file_icon_for_path("NOEXT_zzz"), DEFAULT_FILE_ICON);
        assert_eq!(DEFAULT_FILE_ICON, "material/file.svg");
    }

    #[test]
    fn every_mapped_icon_is_embedded_and_tables_sorted() {
        for table in [FILE_EXTENSIONS, FILE_NAMES] {
            assert!(table.windows(2).all(|w| w[0].0 < w[1].0));
            for (_, icon) in table {
                assert!(material_asset(icon).is_some(), "{icon} missing");
            }
        }
        assert!(MATERIAL_ICONS.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(material_asset(DEFAULT_FILE_ICON).is_some());
    }

    /// GPUI paints `img()` SVGs through resvg: every vendored icon must parse and
    /// draw something with its own colours (not an all-transparent pixmap).
    #[gpui::test]
    fn every_embedded_icon_renders_with_native_colour(cx: &mut gpui::TestAppContext) {
        let renderer = cx.update(|cx| cx.svg_renderer());
        for (path, bytes) in MATERIAL_ICONS {
            let image = gpui::Image::from_bytes(gpui::ImageFormat::Svg, bytes.to_vec())
                .to_image_data(renderer.clone())
                .unwrap_or_else(|e| panic!("{path}: {e}"));
            let pixels = image.as_bytes(0).expect("frame 0");
            assert!(pixels.chunks_exact(4).any(|p| p[3] > 0), "{path} is blank");
        }
    }
}
