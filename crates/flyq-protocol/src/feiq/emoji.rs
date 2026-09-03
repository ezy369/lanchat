//! FeiQ inline emoji codes.
//!
//! FeiQ uses QQ-style text codes embedded in messages:
//! `/:)`, `/:D`, `/:P`, `/:rose`, `/:shake`, etc.
//!
//! These are rendered client-side as images. There are ~100 standard codes.

use serde::{Deserialize, Serialize};

/// Known FeiQ emoji codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EmojiCode {
    // ─── Face emotions ─────────────────────────────────────────────────────
    Smile,        // /:)
    Grin,         // /:D
    Tongue,       // /:P
    Wink,         // /;)
    Cry,          // /:(
    Angry,        // /:@
    Shocked,      // /:O
    Sweat,        // /:sweat
    Cool,         // /:cool
    Shy,          // /:shy
    Sleep,        // /:sleep
    Cry2,         // /:cry
    Laugh,        // /:laugh
    Doubt,        // /:?
    Embarrassed,  // /:embarrassed
    Scared,       // /:scared
    Sick,         // /:sick
    Mask,         // /:mask
    Crazy,        // /:crazy
    Devild,       // /:devild

    // ─── Gestures & actions ────────────────────────────────────────────────
    Shake,        // /:shake (握手)
    Victory,      // /:victory (V手势)
    Love,         // /:love (爱心)
    Heart,        // /:heart
    Kiss,         // /:kiss
    Hug,          // /:hug
    Strong,       // /:strong (大拇指)
    Weak,         // /:weak
    OK,           // /:ok
    Fist,         // /:fist
    Handshake,    // /:handshake

    // ─── Objects & nature ──────────────────────────────────────────────────
    Rose,         // /:rose (玫瑰)
    Sun,          // /:sun
    Moon,         // /:moon
    Star,         // /:star
    Fire,         // /:fire
    Cloud,        // /:cloud
    Rain,         // /:rain
    Lightning,    // /:lightning
    Snow,         // /:snow
    Cake,         // /:cake
    Gift,         // /:gift
    Beer,         // /:beer
    Coffee,       // /:coffee

    // ─── Special ───────────────────────────────────────────────────────────
    Rotate,       // /:<rotate> (旋转动画)
    Unknown,      // Unrecognized code
}

impl EmojiCode {
    /// Get the text representation of this emoji code.
    pub fn as_str(self) -> &'static str {
        match self {
            EmojiCode::Smile => "/:)",
            EmojiCode::Grin => "/:D",
            EmojiCode::Tongue => "/:P",
            EmojiCode::Wink => "/;)",
            EmojiCode::Cry => "/:(",
            EmojiCode::Angry => "/:@",
            EmojiCode::Shocked => "/:O",
            EmojiCode::Sweat => "/:sweat",
            EmojiCode::Cool => "/:cool",
            EmojiCode::Shy => "/:shy",
            EmojiCode::Sleep => "/:sleep",
            EmojiCode::Cry2 => "/:cry",
            EmojiCode::Laugh => "/:laugh",
            EmojiCode::Doubt => "/:?",
            EmojiCode::Embarrassed => "/:embarrassed",
            EmojiCode::Scared => "/:scared",
            EmojiCode::Sick => "/:sick",
            EmojiCode::Mask => "/:mask",
            EmojiCode::Crazy => "/:crazy",
            EmojiCode::Devild => "/:devild",
            EmojiCode::Shake => "/:shake",
            EmojiCode::Victory => "/:victory",
            EmojiCode::Love => "/:love",
            EmojiCode::Heart => "/:heart",
            EmojiCode::Kiss => "/:kiss",
            EmojiCode::Hug => "/:hug",
            EmojiCode::Strong => "/:strong",
            EmojiCode::Weak => "/:weak",
            EmojiCode::OK => "/:ok",
            EmojiCode::Fist => "/:fist",
            EmojiCode::Handshake => "/:handshake",
            EmojiCode::Rose => "/:rose",
            EmojiCode::Sun => "/:sun",
            EmojiCode::Moon => "/:moon",
            EmojiCode::Star => "/:star",
            EmojiCode::Fire => "/:fire",
            EmojiCode::Cloud => "/:cloud",
            EmojiCode::Rain => "/:rain",
            EmojiCode::Lightning => "/:lightning",
            EmojiCode::Snow => "/:snow",
            EmojiCode::Cake => "/:cake",
            EmojiCode::Gift => "/:gift",
            EmojiCode::Beer => "/:beer",
            EmojiCode::Coffee => "/:coffee",
            EmojiCode::Rotate => "/:<rotate>",
            EmojiCode::Unknown => "",
        }
    }

    /// Parse an emoji code from its text representation.
    pub fn from_str(s: &str) -> Self {
        match s {
            "/:)" => EmojiCode::Smile,
            "/:D" => EmojiCode::Grin,
            "/:P" => EmojiCode::Tongue,
            "/;)" => EmojiCode::Wink,
            "/:(" => EmojiCode::Cry,
            "/:@" => EmojiCode::Angry,
            "/:O" => EmojiCode::Shocked,
            "/:sweat" => EmojiCode::Sweat,
            "/:cool" => EmojiCode::Cool,
            "/:shy" => EmojiCode::Shy,
            "/:sleep" => EmojiCode::Sleep,
            "/:cry" => EmojiCode::Cry2,
            "/:laugh" => EmojiCode::Laugh,
            "/:?" => EmojiCode::Doubt,
            "/:embarrassed" => EmojiCode::Embarrassed,
            "/:scared" => EmojiCode::Scared,
            "/:sick" => EmojiCode::Sick,
            "/:mask" => EmojiCode::Mask,
            "/:crazy" => EmojiCode::Crazy,
            "/:devild" => EmojiCode::Devild,
            "/:shake" => EmojiCode::Shake,
            "/:victory" => EmojiCode::Victory,
            "/:love" => EmojiCode::Love,
            "/:heart" => EmojiCode::Heart,
            "/:kiss" => EmojiCode::Kiss,
            "/:hug" => EmojiCode::Hug,
            "/:strong" => EmojiCode::Strong,
            "/:weak" => EmojiCode::Weak,
            "/:ok" => EmojiCode::OK,
            "/:fist" => EmojiCode::Fist,
            "/:handshake" => EmojiCode::Handshake,
            "/:rose" => EmojiCode::Rose,
            "/:sun" => EmojiCode::Sun,
            "/:moon" => EmojiCode::Moon,
            "/:star" => EmojiCode::Star,
            "/:fire" => EmojiCode::Fire,
            "/:cloud" => EmojiCode::Cloud,
            "/:rain" => EmojiCode::Rain,
            "/:lightning" => EmojiCode::Lightning,
            "/:snow" => EmojiCode::Snow,
            "/:cake" => EmojiCode::Cake,
            "/:gift" => EmojiCode::Gift,
            "/:beer" => EmojiCode::Beer,
            "/:coffee" => EmojiCode::Coffee,
            "/:<rotate>" => EmojiCode::Rotate,
            _ => EmojiCode::Unknown,
        }
    }

    /// Get a human-readable description of this emoji.
    pub fn description(self) -> &'static str {
        match self {
            EmojiCode::Smile => "微笑",
            EmojiCode::Grin => "大笑",
            EmojiCode::Tongue => "吐舌",
            EmojiCode::Wink => "眨眼",
            EmojiCode::Cry => "难过",
            EmojiCode::Angry => "生气",
            EmojiCode::Shocked => "惊讶",
            EmojiCode::Sweat => "流汗",
            EmojiCode::Cool => "酷",
            EmojiCode::Shy => "害羞",
            EmojiCode::Sleep => "睡觉",
            EmojiCode::Cry2 => "大哭",
            EmojiCode::Laugh => "偷笑",
            EmojiCode::Doubt => "疑问",
            EmojiCode::Embarrassed => "尴尬",
            EmojiCode::Scared => "惊恐",
            EmojiCode::Sick => "生病",
            EmojiCode::Mask => "口罩",
            EmojiCode::Crazy => "抓狂",
            EmojiCode::Devild => "恶魔",
            EmojiCode::Shake => "握手",
            EmojiCode::Victory => "胜利",
            EmojiCode::Love => "爱心",
            EmojiCode::Heart => "心",
            EmojiCode::Kiss => "亲吻",
            EmojiCode::Hug => "拥抱",
            EmojiCode::Strong => "强",
            EmojiCode::Weak => "弱",
            EmojiCode::OK => "OK",
            EmojiCode::Fist => "拳头",
            EmojiCode::Handshake => "握手",
            EmojiCode::Rose => "玫瑰",
            EmojiCode::Sun => "太阳",
            EmojiCode::Moon => "月亮",
            EmojiCode::Star => "星星",
            EmojiCode::Fire => "火",
            EmojiCode::Cloud => "云",
            EmojiCode::Rain => "雨",
            EmojiCode::Lightning => "闪电",
            EmojiCode::Snow => "雪",
            EmojiCode::Cake => "蛋糕",
            EmojiCode::Gift => "礼物",
            EmojiCode::Beer => "啤酒",
            EmojiCode::Coffee => "咖啡",
            EmojiCode::Rotate => "旋转",
            EmojiCode::Unknown => "未知",
        }
    }
}

/// Extract emoji codes from a message text.
///
/// Returns a list of (position, EmojiCode) tuples.
pub fn extract_emojis(text: &str) -> Vec<(usize, EmojiCode)> {
    let mut emojis = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '/' && i + 1 < chars.len() {
            // Try to match an emoji code
            if chars[i + 1] == ':' || chars[i + 1] == ';' {
                let start = i;
                // Try matching known patterns
                let remaining: String = chars[i..].iter().collect();

                // Check for /:<rotate> special case
                if remaining.starts_with("/:<rotate>") {
                    emojis.push((start, EmojiCode::Rotate));
                    i += "/:<rotate>".chars().count();
                    continue;
                }

                // Check short codes (2-3 chars): /:) /:D /:P /;) /:( /:@ /:O /:?
                let short_len = if chars[i + 1] == ':' || chars[i + 1] == ';' {
                    3 // /:X or /;X
                } else {
                    0
                };

                if short_len > 0 && i + short_len <= chars.len() {
                    let short: String = chars[i..i + short_len].iter().collect();
                    let code = EmojiCode::from_str(&short);
                    if code != EmojiCode::Unknown {
                        emojis.push((start, code));
                        i += short_len;
                        continue;
                    }
                }

                // Check named codes: /:name (word chars only)
                if let Some(end_offset) = find_named_emoji_end(&remaining) {
                    let code_str = &remaining[..end_offset];
                    let code = EmojiCode::from_str(code_str);
                    if code != EmojiCode::Unknown {
                        emojis.push((start, code));
                        i += code_str.chars().count();
                        continue;
                    }
                }
            }
        }
        i += 1;
    }

    emojis
}

/// Find the end of a named emoji code like "/:rose".
fn find_named_emoji_end(s: &str) -> Option<usize> {
    // Skip "/:" prefix
    let after_prefix = &s[2..];
    // Named emojis are word characters (alphanumeric + underscore)
    let end = after_prefix
        .char_indices()
        .find(|(_, c)| !c.is_alphanumeric() && *c != '_')
        .map(|(i, _)| i + 2)
        .unwrap_or(s.len());
    Some(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emoji_roundtrip() {
        let codes = [EmojiCode::Smile, EmojiCode::Grin, EmojiCode::Rose, EmojiCode::Strong];
        for code in codes {
            let text = code.as_str();
            assert_eq!(EmojiCode::from_str(text), code);
        }
    }

    #[test]
    fn test_extract_emojis_simple() {
        let emojis = extract_emojis("Hello /:) World");
        assert_eq!(emojis.len(), 1);
        assert_eq!(emojis[0].1, EmojiCode::Smile);
    }

    #[test]
    fn test_extract_emojis_named() {
        let emojis = extract_emojis("给你一朵 /:rose 花");
        assert_eq!(emojis.len(), 1);
        assert_eq!(emojis[0].1, EmojiCode::Rose);
    }

    #[test]
    fn test_extract_emojis_multiple() {
        let emojis = extract_emojis("/:D /:rose /:strong");
        assert_eq!(emojis.len(), 3);
        assert_eq!(emojis[0].1, EmojiCode::Grin);
        assert_eq!(emojis[1].1, EmojiCode::Rose);
        assert_eq!(emojis[2].1, EmojiCode::Strong);
    }

    #[test]
    fn test_extract_emojis_none() {
        let emojis = extract_emojis("Plain text without emoji");
        assert!(emojis.is_empty());
    }

    #[test]
    fn test_emoji_description() {
        assert_eq!(EmojiCode::Smile.description(), "微笑");
        assert_eq!(EmojiCode::Rose.description(), "玫瑰");
    }
}
