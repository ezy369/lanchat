//! FeiQ rich text format.
//!
//! FeiQ messages can contain rich text formatting:
//! ```text
//! message text{/font;-14 0 0 0 0 400 0 0 0 1 0 34 3 2 1 2 宋体
//! ```
//!
//! The format block contains LOGFONT-like parameters:
//! - height, width, escapement, orientation, weight
//! - italic, underline, strikeout, charset, outprecision
//! - clipprecision, quality, pitchandfamily, facename

use serde::{Deserialize, Serialize};

/// Parsed rich text with formatting information.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RichText {
    /// Plain text content.
    pub text: String,
    /// Font formatting (if present).
    pub font: Option<FontFormat>,
}

/// Font formatting information from FeiQ rich text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontFormat {
    /// Font height (negative = point size * 20 / dpi).
    pub height: i32,
    /// Font width (0 = default).
    pub width: i32,
    /// Font weight (400 = normal, 700 = bold).
    pub weight: i32,
    /// Italic flag (0 or 1).
    pub italic: u8,
    /// Underline flag (0 or 1).
    pub underline: u8,
    /// Strikeout flag (0 or 1).
    pub strikeout: u8,
    /// Character set.
    pub charset: u8,
    /// Font family name.
    pub family: String,
}

impl FontFormat {
    /// Create a default font format (宋体, normal weight).
    pub fn default_chinese() -> Self {
        Self {
            height: -14,
            width: 0,
            weight: 400,
            italic: 0,
            underline: 0,
            strikeout: 0,
            charset: 2,
            family: "宋体".to_string(),
        }
    }

    /// Create a bold font format.
    pub fn bold() -> Self {
        Self {
            weight: 700,
            ..Self::default_chinese()
        }
    }

    /// Check if this font is bold.
    pub fn is_bold(&self) -> bool {
        self.weight >= 700
    }

    /// Check if this font is italic.
    pub fn is_italic(&self) -> bool {
        self.italic != 0
    }

    /// Check if this font has underline.
    pub fn is_underline(&self) -> bool {
        self.underline != 0
    }

    /// Approximate point size from height.
    pub fn point_size(&self) -> f64 {
        (-self.height as f64) * 72.0 / 96.0
    }
}

impl RichText {
    /// Parse a FeiQ rich text message.
    ///
    /// Format: `text{/font;height width 0 0 0 weight
    ///  italic underline strikeout charset outprecision clipprecision
    ///  quality pitchandfamily extra1 extra2 extra3 facename`
    pub fn parse(raw: &str) -> Self {
        // Look for the font format marker
        if let Some(marker_pos) = raw.find("{/font;") {
            let text = raw[..marker_pos].to_string();
            let format_str = &raw[marker_pos + 7..]; // Skip "{/font;"

            let parts: Vec<&str> = format_str.split_whitespace().collect();
            if parts.len() >= 10 {
                // FeiQ format indices (0-based):
                // 0=height, 1=width, 2=escapement, 3=orientation, 4=unknown,
                // 5=weight, 6=italic, 7=underline, 8=strikeout, 9=charset,
                // 10..15=fixed, 16+=facename
                let font = FontFormat {
                    height: parts[0].parse().unwrap_or(-14),
                    width: parts[1].parse().unwrap_or(0),
                    weight: parts.get(5).and_then(|s| s.parse().ok()).unwrap_or(400),
                    italic: parts.get(6).and_then(|s| s.parse().ok()).unwrap_or(0),
                    underline: parts.get(7).and_then(|s| s.parse().ok()).unwrap_or(0),
                    strikeout: parts.get(8).and_then(|s| s.parse().ok()).unwrap_or(0),
                    charset: parts.get(9).and_then(|s| s.parse().ok()).unwrap_or(2),
                    family: if parts.len() > 16 {
                        parts[16..].join(" ")
                    } else {
                        parts.last().unwrap_or(&"宋体").to_string()
                    },
                };
                return Self {
                    text,
                    font: Some(font),
                };
            }

            return Self { text, font: None };
        }

        Self {
            text: raw.to_string(),
            font: None,
        }
    }

    /// Build a rich text message with font formatting.
    pub fn build(text: &str, font: &FontFormat) -> String {
        format!(
            "{}{{/font;{} {} 0 0 0 {} {} {} {} {} 1 0 34 3 2 1 2 {}",
            text, font.height, font.width, font.weight, font.italic,
            font.underline, font.strikeout, font.charset, font.family
        )
    }

    /// Build a rich text string from this RichText.
    pub fn to_wire_format(&self) -> String {
        match &self.font {
            Some(font) => format!(
                "{}{{/font;{} {} 0 0 0 {} {} {} {} {} 1 0 34 3 2 1 2 {}",
                self.text, font.height, font.width, font.weight, font.italic,
                font.underline, font.strikeout, font.charset, font.family
            ),
            None => self.text.clone(),
        }
    }

    /// Create a plain text RichText (no formatting).
    pub fn plain(text: &str) -> Self {
        Self {
            text: text.to_string(),
            font: None,
        }
    }

    /// Check if this message has any formatting.
    pub fn has_formatting(&self) -> bool {
        self.font.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_plain_text() {
        let rt = RichText::parse("Hello World");
        assert_eq!(rt.text, "Hello World");
        assert!(!rt.has_formatting());
    }

    #[test]
    fn test_parse_rich_text() {
        let raw = "测试消息{/font;-14 0 0 0 0 400 0 0 0 1 0 34 3 2 1 2 宋体";
        let rt = RichText::parse(raw);
        assert_eq!(rt.text, "测试消息");
        assert!(rt.has_formatting());
        let font = rt.font.unwrap();
        assert_eq!(font.height, -14);
        assert_eq!(font.weight, 400);
        assert!(!font.is_bold());
    }

    #[test]
    fn test_parse_bold_text() {
        let raw = "Bold{/font;-14 0 0 0 0 700 0 0 0 1 0 34 3 2 1 2 微软雅黑";
        let rt = RichText::parse(raw);
        assert_eq!(rt.text, "Bold");
        let font = rt.font.unwrap();
        assert_eq!(font.weight, 700);
        assert!(font.is_bold());
        assert_eq!(font.family, "微软雅黑");
    }

    #[test]
    fn test_font_point_size() {
        let font = FontFormat::default_chinese();
        assert!((font.point_size() - 10.5).abs() < 0.1);
    }

    #[test]
    fn test_rich_text_to_wire() {
        let rt = RichText {
            text: "Hello".to_string(),
            font: Some(FontFormat::default_chinese()),
        };
        let wire = rt.to_wire_format();
        assert!(wire.starts_with("Hello{/font;"));
        assert!(wire.contains("-14"));
        assert!(wire.contains("宋体"));
    }
}
