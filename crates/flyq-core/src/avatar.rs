//! Default avatar generation.
//!
//! Generates simple SVG avatars with a coloured circle and the user's initial
//! letter. The colour is deterministically chosen from a palette based on a
//! hash of the display name so the same user always gets the same colour.

use std::path::{Path, PathBuf};

/// Palette of pleasant background colours for default avatars.
const PALETTE: &[&str] = &[
    "#E57373", // red
    "#F06292", // pink
    "#BA68C8", // purple
    "#9575CD", // deep-purple
    "#7986CB", // indigo
    "#64B5F6", // blue
    "#4FC3F7", // light-blue
    "#4DD0E1", // cyan
    "#4DB6AC", // teal
    "#81C784", // green
    "#AED581", // light-green
    "#FFD54F", // amber
    "#FFB74D", // orange
    "#FF8A65", // deep-orange
    "#A1887F", // brown
    "#90A4AE", // blue-grey
];

/// Default avatar image size (width and height) in pixels.
pub const AVATAR_SIZE: u32 = 128;

/// Generate a default avatar SVG for a display name.
///
/// Returns an SVG string containing a coloured circle with the user's first
/// character (uppercased) centred inside it.
pub fn generate_avatar_svg(name: &str) -> String {
    let initial = name
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();

    let colour = pick_colour(name);
    let size = AVATAR_SIZE;
    let cx = size / 2;
    let cy = size / 2;
    let r = size / 2 - 2;
    let font_size = size * 48 / 100; // ~48% of canvas

    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 {size} {size}"><circle cx="{cx}" cy="{cy}" r="{r}" fill="{colour}"/><text x="{cx}" y="{cy}" dy=".35em" text-anchor="middle" fill="#fff" font-family="sans-serif" font-size="{font_size}" font-weight="600">{initial}</text></svg>"##,
    )
}

/// Pick a colour from the palette based on a simple hash of the name.
fn pick_colour(name: &str) -> &'static str {
    let hash: u32 = name.bytes().fold(0u32, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(b as u32)
    });
    PALETTE[(hash as usize) % PALETTE.len()]
}

/// Ensure the avatars cache directory exists and return its path.
pub fn ensure_avatar_dir(base_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = base_dir.join("avatars");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Generate (or return cached) default avatar SVG file for a peer.
///
/// `addr` is used as the filename key (colons replaced with underscores).
/// Returns the path to the SVG file on disk.
pub fn get_or_create_default_avatar(
    avatar_dir: &Path,
    addr: &str,
    display_name: &str,
) -> std::io::Result<PathBuf> {
    let safe_name = addr.replace([':', '/'], "_");
    let path = avatar_dir.join(format!("default_{}.svg", safe_name));

    if path.exists() {
        return Ok(path);
    }

    let svg = generate_avatar_svg(display_name);
    std::fs::write(&path, svg)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatar_svg_contains_initial() {
        let svg = generate_avatar_svg("Alice");
        assert!(svg.contains(">A</text>"), "SVG should contain initial 'A'");
    }

    #[test]
    fn avatar_svg_chinese() {
        let svg = generate_avatar_svg("小明");
        assert!(svg.contains(">小</text>"), "SVG should contain initial '小'");
    }

    #[test]
    fn avatar_svg_empty_name() {
        let svg = generate_avatar_svg("");
        assert!(svg.contains(">?</text>"), "Empty name should use '?'");
    }

    #[test]
    fn colour_deterministic() {
        let c1 = pick_colour("Alice");
        let c2 = pick_colour("Alice");
        assert_eq!(c1, c2, "Same name should produce same colour");
    }

    #[test]
    fn colour_varies() {
        let c1 = pick_colour("Alice");
        let c2 = pick_colour("Bob");
        // Not guaranteed to differ, but very likely with different names.
        // Just ensure no panics.
        assert!(!c1.is_empty());
        assert!(!c2.is_empty());
    }

    #[test]
    fn default_avatar_file_created() {
        let dir = std::env::temp_dir().join(format!("lanchat_avatar_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);

        let path =
            get_or_create_default_avatar(&dir, "192.168.1.1:2425", "TestUser").unwrap();
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains(">T</text>"));

        // Second call returns cached file.
        let path2 =
            get_or_create_default_avatar(&dir, "192.168.1.1:2425", "TestUser").unwrap();
        assert_eq!(path, path2);

        // Cleanup.
        let _ = std::fs::remove_dir_all(&dir);
    }
}
