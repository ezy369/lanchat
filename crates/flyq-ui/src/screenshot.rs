//! Screenshot capture and crop engine.
//!
//! Uses `xcap` to grab the primary monitor and saves a PNG to the local
//! images cache directory. Supports rectangular region cropping via
//! [`crop_image`]. Called from a blocking Tokio worker so the
//! capture never stalls the GPUI render thread.

use std::path::{Path, PathBuf};

/// Capture the primary monitor and save a PNG to `images_dir`.
///
/// Returns the saved file path on success. The filename is
/// `screenshot_{timestamp}.png` to avoid collisions.
pub fn capture_and_save(images_dir: &Path) -> Result<PathBuf, String> {
    // Ensure the images directory exists.
    std::fs::create_dir_all(images_dir)
        .map_err(|e| format!("无法创建截图目录: {}", e))?;

    // Grab the primary monitor.
    let monitors = xcap::Monitor::all().map_err(|e| format!("无法枚举显示器: {}", e))?;
    let monitor = monitors.first().ok_or_else(|| "未找到显示器".to_string())?;

    let image = monitor
        .capture_image()
        .map_err(|e| format!("截图失败: {}", e))?;

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let filename = format!("screenshot_{}.png", ts);
    let dest = images_dir.join(&filename);

    image
        .save(&dest)
        .map_err(|e| format!("保存截图失败: {}", e))?;

    Ok(dest)
}

/// Capture the primary monitor and save to a temporary file in `temp_dir`.
///
/// Returns the path to the temporary screenshot file. The caller should
/// clean up this file when done (or it will be overwritten on next capture).
pub fn capture_to_temp(temp_dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(temp_dir)
        .map_err(|e| format!("无法创建临时目录: {}", e))?;

    let monitors = xcap::Monitor::all().map_err(|e| format!("无法枚举显示器: {}", e))?;
    let monitor = monitors.first().ok_or_else(|| "未找到显示器".to_string())?;

    let img = monitor
        .capture_image()
        .map_err(|e| format!("截图失败: {}", e))?;

    let dest = temp_dir.join("screenshot_temp.png");
    img.save(&dest)
        .map_err(|e| format!("保存临时截图失败: {}", e))?;

    Ok(dest)
}

/// Crop a rectangular region from an image file and save the result.
///
/// Coordinates are in pixels. Out-of-bounds values are clamped to the image
/// dimensions. Returns the path to the cropped image file.
pub fn crop_image(
    source: &Path,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    dest: &Path,
) -> Result<PathBuf, String> {
    let img = image::open(source).map_err(|e| format!("打开图片失败: {}", e))?;
    let (iw, ih) = (img.width(), img.height());

    // Reject origin beyond image bounds.
    if x >= iw || y >= ih {
        return Err("裁剪区域起点超出图片范围".to_string());
    }

    // Clamp width/height to remaining image area.
    let w = width.min(iw - x);
    let h = height.min(ih - y);

    if w == 0 || h == 0 {
        return Err("裁剪区域无效".to_string());
    }

    let cropped = img.crop_imm(x, y, w, h);
    cropped
        .save(dest)
        .map_err(|e| format!("保存裁剪图片失败: {}", e))?;

    Ok(dest.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a small test image and return its path.
    fn create_test_image(dir: &Path) -> PathBuf {
        let img = image::RgbaImage::from_fn(100, 80, |x, y| {
            image::Rgba([x as u8, y as u8, 128, 255])
        });
        let path = dir.join("test_input.png");
        img.save(&path).unwrap();
        path
    }

    #[test]
    fn test_crop_basic() {
        let dir = std::env::temp_dir().join("flyq_test_crop_basic");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let src = create_test_image(&dir);
        let dest = dir.join("cropped.png");

        let result = crop_image(&src, 10, 10, 50, 40, &dest);
        assert!(result.is_ok());

        let cropped = image::open(&dest).unwrap();
        assert_eq!(cropped.width(), 50);
        assert_eq!(cropped.height(), 40);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_crop_clamp_to_bounds() {
        let dir = std::env::temp_dir().join("flyq_test_crop_clamp");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let src = create_test_image(&dir); // 100x80
        let dest = dir.join("cropped.png");

        // Request region extends beyond image — should be clamped.
        let result = crop_image(&src, 80, 60, 50, 50, &dest);
        assert!(result.is_ok());

        let cropped = image::open(&dest).unwrap();
        assert_eq!(cropped.width(), 20); // 100 - 80
        assert_eq!(cropped.height(), 20); // 80 - 60

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_crop_zero_dimensions() {
        let dir = std::env::temp_dir().join("flyq_test_crop_zero");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let src = create_test_image(&dir);
        let dest = dir.join("cropped.png");

        let result = crop_image(&src, 0, 0, 0, 0, &dest);
        assert!(result.is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_crop_origin_beyond_image() {
        let dir = std::env::temp_dir().join("flyq_test_crop_origin");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let src = create_test_image(&dir); // 100x80
        let dest = dir.join("cropped.png");

        // Origin beyond image bounds.
        let result = crop_image(&src, 200, 200, 50, 50, &dest);
        assert!(result.is_err()); // width would be 0 after clamping

        let _ = fs::remove_dir_all(&dir);
    }
}
