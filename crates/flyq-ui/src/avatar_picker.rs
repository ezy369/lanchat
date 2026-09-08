//! Avatar image preparation for the settings panel.
//!
//! Provides [`prepare_avatar`] which loads a user-selected image, crops it to
//! the largest centered square, resizes to 128×128 pixels, and saves it to the
//! designated cache path. Runs on a blocking worker to avoid stalling the UI.

use std::path::{Path, PathBuf};

/// Target avatar size in pixels (matches flyq-core AVATAR_SIZE).
const AVATAR_SIZE: u32 = 128;

/// Load an image from `source`, crop to the largest centered square, resize to
/// 128×128 and save as PNG to `dest`.
///
/// Returns the destination path on success.
pub fn prepare_avatar(source: &Path, dest: &Path) -> Result<PathBuf, String> {
    let img = image::open(source).map_err(|e| format!("打开图片失败: {}", e))?;
    let (w, h) = (img.width(), img.height());

    // Crop to largest centered square.
    let side = w.min(h);
    let x = (w - side) / 2;
    let y = (h - side) / 2;
    let cropped = img.crop_imm(x, y, side, side);

    // Resize to target size using high-quality Lanczos3.
    let resized = cropped.resize_exact(
        AVATAR_SIZE,
        AVATAR_SIZE,
        image::imageops::FilterType::Lanczos3,
    );

    // Ensure parent directory exists.
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败: {}", e))?;
    }

    resized
        .save(dest)
        .map_err(|e| format!("保存头像失败: {}", e))?;

    Ok(dest.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a test image with the given dimensions.
    fn create_test_image(dir: &Path, w: u32, h: u32, name: &str) -> PathBuf {
        let img = image::RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([x as u8, y as u8, 128, 255])
        });
        let path = dir.join(name);
        img.save(&path).unwrap();
        path
    }

    #[test]
    fn test_prepare_avatar_square_source() {
        let dir = std::env::temp_dir().join("lanchat_avatar_test_sq");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let src = create_test_image(&dir, 256, 256, "square.png");
        let dest = dir.join("avatar.png");
        let result = prepare_avatar(&src, &dest);
        assert!(result.is_ok(), "prepare_avatar failed: {:?}", result.err());

        let out = image::open(&dest).unwrap();
        assert_eq!(out.width(), AVATAR_SIZE);
        assert_eq!(out.height(), AVATAR_SIZE);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_prepare_avatar_landscape_source() {
        let dir = std::env::temp_dir().join("lanchat_avatar_test_land");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let src = create_test_image(&dir, 400, 200, "landscape.png");
        let dest = dir.join("avatar.png");
        let result = prepare_avatar(&src, &dest);
        assert!(result.is_ok());

        let out = image::open(&dest).unwrap();
        assert_eq!(out.width(), AVATAR_SIZE);
        assert_eq!(out.height(), AVATAR_SIZE);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_prepare_avatar_portrait_source() {
        let dir = std::env::temp_dir().join("lanchat_avatar_test_port");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let src = create_test_image(&dir, 150, 500, "portrait.png");
        let dest = dir.join("avatar.png");
        let result = prepare_avatar(&src, &dest);
        assert!(result.is_ok());

        let out = image::open(&dest).unwrap();
        assert_eq!(out.width(), AVATAR_SIZE);
        assert_eq!(out.height(), AVATAR_SIZE);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_prepare_avatar_small_source() {
        let dir = std::env::temp_dir().join("lanchat_avatar_test_small");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Source smaller than target — should still upscale to 128x128.
        let src = create_test_image(&dir, 32, 32, "tiny.png");
        let dest = dir.join("avatar.png");
        let result = prepare_avatar(&src, &dest);
        assert!(result.is_ok());

        let out = image::open(&dest).unwrap();
        assert_eq!(out.width(), AVATAR_SIZE);
        assert_eq!(out.height(), AVATAR_SIZE);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_prepare_avatar_missing_source() {
        let dest = std::env::temp_dir().join("lanchat_avatar_test_missing_out.png");
        let result = prepare_avatar(Path::new("/nonexistent/image.png"), &dest);
        assert!(result.is_err());
    }
}
