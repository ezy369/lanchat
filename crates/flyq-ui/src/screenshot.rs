//! Screenshot capture engine.
//!
//! Uses `xcap` to grab the primary monitor and saves a PNG to the local
//! images cache directory. Called from a blocking Tokio worker so the
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
