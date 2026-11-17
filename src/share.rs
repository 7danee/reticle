//! Crosshair codes: reticle's own share code plus import of Counter-Strike and Valorant codes.
//! Everything works offline; a code is just text to paste into Discord or a chat.
//!
//! - reticle: `rtcl1:` + base64url(deflate(JSON of the crosshair)). Image layers stay local.
//! - Counter-Strike: `CS…` (CS2, pixel based) and `CSGO-…` (legacy v1, v3, v4). Layout as
//!   documented by the MIT-licensed csgo-sharecode project.
//! - Valorant: `0;P;c;5;…` key/value list, primary crosshair only.
//!
//! Game codes describe sizes for a reference resolution; they are scaled to `screen_height`.

use miniz_oxide::deflate::compress_to_vec;
use miniz_oxide::inflate::decompress_to_vec_with_limit;

use crate::config::{
    CrossLayer, Crosshair, DotLayer, LIMIT_DOT, LIMIT_GAP, LIMIT_LENGTH, LIMIT_OUTLINE, LIMIT_THICKNESS, Layer, Outline,
    Rgba,
};

const PREFIX: &str = "rtcl1:";
/// Inflated codes larger than this are rejected.
const MAX_JSON: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Reticle,
    CounterStrike,
    Valorant,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Reticle => "reticle",
            Source::CounterStrike => "Counter-Strike",
            Source::Valorant => "Valorant",
        }
    }
}

/// Share code for a crosshair. Returns whether image layers had to be left out.
pub fn encode(crosshair: &Crosshair) -> (String, bool) {
    let layers: Vec<Layer> =
        crosshair.layers.iter().filter(|l| !matches!(l, Layer::Image(_))).cloned().collect();
    let dropped = layers.len() != crosshair.layers.len();
    let json = serde_json::to_vec(&Crosshair { layers }).expect("crosshair serializes");
    (format!("{PREFIX}{}", base64url_encode(&compress_to_vec(&json, 9))), dropped)
}

/// Reads any supported code. `screen_height` is the height of the monitor the overlay uses.
pub fn decode(code: &str, screen_height: u32) -> Result<(Crosshair, Source), String> {
    let code: String = code.split_whitespace().collect();
    let height = screen_height.max(240) as f32;
    if let Some(body) = code.strip_prefix(PREFIX) {
        return decode_reticle(body).map(|c| (c, Source::Reticle));
    }
    if code.starts_with("CS") {
        return decode_counter_strike(&code, height).map(|c| (c, Source::CounterStrike));
    }
    if code.contains(';') {
        return decode_valorant(&code, height).map(|c| (c, Source::Valorant));
    }
    Err("Unbekannter Code. Erlaubt sind reticle-, Counter-Strike- und Valorant-Codes.".into())
}

fn decode_reticle(body: &str) -> Result<Crosshair, String> {
    let bad = || "Der reticle-Code ist beschädigt oder unvollständig.".to_string();
    let packed = base64url_decode(body).ok_or_else(bad)?;
    let json = decompress_to_vec_with_limit(&packed, MAX_JSON).map_err(|_| bad())?;
    let crosshair: Crosshair = serde_json::from_slice(&json).map_err(|_| bad())?;
    crosshair.validate()?;
    if crosshair.layers.is_empty() {
        return Err(bad());
    }
    Ok(crosshair)
}

// ---------------------------------------------------------------------------------------------
// Counter-Strike

const CS_DICTIONARY: &[u8] = b"ABCDEFGHJKLMNOPQRSTUVWXYZabcdefhijkmnopqrstuvwxyz23456789";

/// Base-57 text to a big-endian byte array. The first character is the least significant digit.
fn cs_bytes(body: &str, len: usize) -> Option<Vec<u8>> {
    let mut bytes = vec![0u8; len];
    for c in body.bytes().rev() {
        let digit = CS_DICTIONARY.iter().position(|&d| d == c)? as u32;
        let mut carry = digit;
        for b in bytes.iter_mut().rev() {
            let v = *b as u32 * 57 + carry;
            *b = (v & 0xFF) as u8;
            carry = v >> 8;
        }
        if carry != 0 {
            return None;
        }
    }
    Some(bytes)
}

fn checksum_ok(bytes: &[u8]) -> bool {
    bytes[1..].iter().fold(0u8, |a, &b| a.wrapping_add(b)) == bytes[0]
}

/// Fields shared by all Counter-Strike formats, sizes already in screen pixels.
struct CsCrosshair {
    color: Rgba,
    thickness: f32,
    length: f32,
    gap: f32,
    t_style: bool,
    dot: bool,
    outline: Option<(f32, Rgba)>,
}

fn decode_counter_strike(code: &str, height: f32) -> Result<Crosshair, String> {
    let invalid = || "Ungültiger Counter-Strike-Code.".to_string();
    let (bytes, legacy) = if let Some(body) = code.strip_prefix("CSGO-") {
        let body: String = body.chars().filter(|&c| c != '-').collect();
        if body.len() != 25 {
            return Err(invalid());
        }
        (cs_bytes(&body, 18).ok_or_else(invalid)?, true)
    } else {
        let body = &code[2..];
        if body.len() != 44 {
            return Err(invalid());
        }
        (cs_bytes(body, 32).ok_or_else(invalid)?, false)
    };
    if !checksum_ok(&bytes) {
        return Err(invalid());
    }
    let b = &bytes;
    let le16 = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let le32 = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let black = Rgba { r: 0, g: 0, b: 0, a: 255 };
    let cs = match (legacy, b[1]) {
        (false, 1) => {
            let screen = (le16(2) as f32).max(240.0);
            if le16(2) == 0 {
                return Err(invalid());
            }
            let k = height / screen;
            let outline_mode = (b[13] >> 6).min(2);
            CsCrosshair {
                color: Rgba { r: b[5], g: b[6], b: b[7], a: b[8] },
                thickness: (b[13] & 0x3F).min(32) as f32 * k,
                length: b[16] as f32 * k,
                gap: (i16::from_le_bytes([b[14], b[15]]).clamp(-3840, 3840)) as f32 * k,
                t_style: b[4] & 0x80 != 0,
                dot: b[4] & 0x40 != 0,
                outline: (outline_mode > 0).then_some((1.0, Rgba { r: b[9], g: b[10], b: b[11], a: b[12] })),
            }
        }
        (true, 1) => {
            // Old arbitrary units: sizes scale with screen height / 480 like the Source HUD.
            let k = height / 480.0;
            let size = b[14] as f32 / 10.0;
            let thickness = b[12] as f32 / 10.0;
            let gap = b[2] as i8 as f32 / 10.0;
            let color = match b[10] & 7 {
                0 => Rgba { r: 250, g: 50, b: 50, a: 255 },
                1 => Rgba { r: 50, g: 250, b: 50, a: 255 },
                2 => Rgba { r: 250, g: 250, b: 50, a: 255 },
                3 => Rgba { r: 50, g: 50, b: 250, a: 255 },
                4 => Rgba { r: 50, g: 250, b: 250, a: 255 },
                _ => Rgba { r: b[4], g: b[5], b: b[6], a: 255 },
            };
            let alpha = if b[13] & 0x40 != 0 { b[7] } else { 255 };
            CsCrosshair {
                color: Rgba { a: alpha, ..color },
                thickness: thickness * k,
                length: size * k,
                gap: (4.0 * height / 1200.0 + gap).ceil(),
                t_style: b[13] & 0x80 != 0,
                dot: b[13] & 0x10 != 0,
                outline: (b[10] & 8 != 0).then_some(((b[3] as f32 / 2.0).max(1.0), black)),
            }
        }
        (true, v @ (3 | 4)) => {
            let screen = (le16(14) as f32).max(240.0);
            let k = height / screen;
            let outline = if v == 3 { b[2] & 0x20 != 0 } else { (b[13] >> 4) & 3 != 0 };
            CsCrosshair {
                color: Rgba { r: b[3], g: b[4], b: b[5], a: b[6] },
                thickness: ((le32(10) >> 23) & 0x1F) as f32 * k,
                length: b[8] as f32 * k,
                gap: b[7] as f32 * k,
                t_style: b[2] & 0x80 != 0,
                dot: b[2] & 0x40 != 0,
                outline: outline.then_some((1.0, black)),
            }
        }
        _ => return Err("Diese Counter-Strike-Code-Version wird nicht unterstützt.".into()),
    };
    Ok(cs.into_crosshair())
}

impl CsCrosshair {
    fn into_crosshair(self) -> Crosshair {
        let px = |v: f32, min: u32, limit: (u32, u32)| (v.round().max(min as f32) as u32).clamp(limit.0, limit.1);
        let thickness = px(self.thickness, 1, LIMIT_THICKNESS);
        let outline = match self.outline {
            Some((w, color)) => Outline { enabled: true, thickness: px(w, 1, LIMIT_OUTLINE), color: color.to_hex() },
            None => Outline::default(),
        };
        let color = self.color.to_hex();
        let length = px(self.length, 0, LIMIT_LENGTH);
        let mut layers = Vec::new();
        if length > 0 {
            layers.push(Layer::Cross(CrossLayer {
                visible: true,
                color: color.clone(),
                thickness,
                length,
                vertical_length: None,
                gap: (self.gap.round() as i32).clamp(LIMIT_GAP.0, LIMIT_GAP.1),
                t_style: self.t_style,
                outline: outline.clone(),
            }));
        }
        if self.dot || layers.is_empty() {
            layers.push(Layer::Dot(DotLayer { visible: true, color, size: thickness.clamp(LIMIT_DOT.0, LIMIT_DOT.1), outline }));
        }
        Crosshair { layers }
    }
}

// ---------------------------------------------------------------------------------------------
// Valorant

/// Valorant's preset colors, by index.
const VALORANT_COLORS: [&str; 8] =
    ["#FFFFFF", "#00FF00", "#7FFF00", "#DFFF00", "#FFFF00", "#00FFFF", "#FF00FF", "#FF0000"];

fn decode_valorant(code: &str, height: f32) -> Result<Crosshair, String> {
    let invalid = || "Ungültiger Valorant-Code.".to_string();
    let tokens: Vec<&str> = code.split(';').filter(|t| !t.is_empty()).collect();
    let mut values: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    let mut section = "";
    // The first token is a format version ("0"), not a key.
    let mut i = 1;
    while i < tokens.len() {
        match tokens[i] {
            "P" | "A" | "S" => {
                section = tokens[i];
                i += 1;
            }
            key if i + 1 < tokens.len() => {
                if section == "P" {
                    values.insert(key, tokens[i + 1]);
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    if section.is_empty() {
        return Err(invalid());
    }
    let num = |key: &str, default: f32| values.get(key).and_then(|v| v.parse::<f32>().ok()).unwrap_or(default);
    let flag = |key: &str, default: bool| values.get(key).map_or(default, |v| *v == "1");

    let base = match num("c", 0.0) as usize {
        8 => values.get("u").and_then(|u| Rgba::parse(&format!("#{u}"))).ok_or_else(invalid)?,
        i => Rgba::parse(VALORANT_COLORS.get(i).copied().unwrap_or("#FFFFFF")).expect("valid preset"),
    };
    let alpha = |opacity: f32| Rgba { a: (base.a as f32 * opacity.clamp(0.0, 1.0)).round() as u8, ..base }.to_hex();
    let k = height / 1080.0;
    let px = |v: f32, limit: (u32, u32)| ((v * k).round().max(limit.0 as f32) as u32).min(limit.1);
    let outline = if flag("h", true) {
        let opacity = num("o", 0.5).clamp(0.0, 1.0);
        Outline {
            enabled: opacity > 0.0,
            thickness: px(num("t", 1.0), LIMIT_OUTLINE),
            color: Rgba { r: 0, g: 0, b: 0, a: (opacity * 255.0).round() as u8 }.to_hex(),
        }
    } else {
        Outline::default()
    };

    let mut layers = Vec::new();
    // Outer lines (prefix 1) sit below the inner lines (prefix 0).
    for (prefix, show, thickness, length, offset, opacity) in
        [("1", true, 2.0, 2.0, 10.0, 0.35), ("0", true, 2.0, 6.0, 3.0, 0.8)]
    {
        let key = |k: &str| format!("{prefix}{k}");
        let opacity = num(&key("a"), opacity);
        let horizontal = num(&key("l"), length);
        let length = px(horizontal, (0, LIMIT_LENGTH.1));
        // "g;1" unlinks the vertical length, which then comes from "v".
        let vertical_length =
            flag(&key("g"), false).then(|| px(num(&key("v"), horizontal), (0, LIMIT_LENGTH.1)));
        if !flag(&key("b"), show) || opacity <= 0.0 || (length == 0 && vertical_length.unwrap_or(0) == 0) {
            continue;
        }
        layers.push(Layer::Cross(CrossLayer {
            visible: true,
            color: alpha(opacity),
            thickness: px(num(&key("t"), thickness), LIMIT_THICKNESS),
            length,
            vertical_length,
            gap: ((num(&key("o"), offset) * k).round() as i32).clamp(LIMIT_GAP.0, LIMIT_GAP.1),
            t_style: false,
            outline: outline.clone(),
        }));
    }
    if flag("d", false) {
        layers.push(Layer::Dot(DotLayer {
            visible: true,
            color: alpha(num("a", 1.0)),
            size: px(num("z", 2.0), LIMIT_DOT),
            outline,
        }));
    }
    if layers.is_empty() {
        return Err("Der Valorant-Code beschreibt ein unsichtbares Crosshair.".into());
    }
    Ok(Crosshair { layers })
}

// ---------------------------------------------------------------------------------------------
// base64url without padding

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64url_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, &b)| acc | (b as u32) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

fn base64url_decode(text: &str) -> Option<Vec<u8>> {
    let digits: Vec<u32> =
        text.trim_end_matches('=').bytes().map(|c| B64.iter().position(|&d| d == c).map(|p| p as u32)).collect::<Option<_>>()?;
    if digits.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(digits.len() * 3 / 4);
    for chunk in digits.chunks(4) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, &d)| acc | d << (18 - 6 * i));
        for i in 0..chunk.len() - 1 {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CircleLayer, ImageLayer};

    fn cross(c: &Crosshair) -> &CrossLayer {
        c.layers.iter().find_map(|l| if let Layer::Cross(x) = l { Some(x) } else { None }).expect("cross layer")
    }

    fn has_dot(c: &Crosshair) -> bool {
        c.layers.iter().any(|l| matches!(l, Layer::Dot(_)))
    }

    #[test]
    fn base64_round_trips() {
        for len in 0..40 {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(base64url_decode(&base64url_encode(&data)).unwrap(), data);
        }
        assert!(base64url_decode("a").is_none());
        assert!(base64url_decode("ab$c").is_none());
    }

    #[test]
    fn reticle_code_round_trips_without_images() {
        let original = Crosshair {
            layers: vec![
                Layer::Circle(CircleLayer::default()),
                Layer::Image(ImageLayer { file: "x.png".into(), ..ImageLayer::default() }),
                Layer::Cross(CrossLayer { gap: -2, ..CrossLayer::default() }),
            ],
        };
        let (code, dropped) = encode(&original);
        assert!(dropped);
        assert!(code.starts_with("rtcl1:") && code.len() < 300, "{code}");
        let (decoded, source) = decode(&format!("  {code}\n"), 1080).unwrap();
        assert_eq!(source, Source::Reticle);
        assert_eq!(decoded.layers.len(), 2);
        assert_eq!(decoded.layers[1], original.layers[2]);
        assert!(decode("rtcl1:AAAA", 1080).is_err());
        assert!(decode(&code[..code.len() - 5], 1080).is_err());
    }

    #[test]
    fn cs2_v1_vector() {
        // From csgo-sharecode's tests: red, length 5, thickness 1, gap 0, center dot, 768p.
        let (c, source) = decode("CSvbPubOq37zTGqtsPTP5QTrp5CB4xFXiKRLfzJsm49ZRe", 768).unwrap();
        assert_eq!(source, Source::CounterStrike);
        let x = cross(&c);
        assert_eq!((x.color.as_str(), x.length, x.thickness, x.gap, x.t_style), ("#FF0000", 5, 1, 0, false));
        assert!(!x.outline.enabled);
        assert!(has_dot(&c));
        // Scaled to a 1536 px tall screen everything doubles.
        let (c, _) = decode("CSvbPubOq37zTGqtsPTP5QTrp5CB4xFXiKRLfzJsm49ZRe", 1536).unwrap();
        assert_eq!((cross(&c).length, cross(&c).thickness), (10, 2));
    }

    #[test]
    fn cs2_v1_full_vector() {
        let (c, _) = decode("CSAB6Oby2JUDOrBRy4jRcsR6JOQdhbszKF9GuKQGhKVwAc", 1440).unwrap();
        let x = cross(&c);
        assert_eq!(x.color, "#0A141EC8");
        assert_eq!((x.length, x.thickness, x.gap, x.t_style), (200, 12, -7, true));
        assert!(x.outline.enabled);
        assert_eq!(x.outline.color, "#28323C80");
        assert!(!has_dot(&c));
    }

    #[test]
    fn legacy_vectors() {
        // legacy v3: pink, gap 4, length 8, thickness 3, outline, dot, 1080p.
        let (c, _) = decode("CSGO-MnUCC-89iG7-2cVar-wy7Yn-amCpF", 1080).unwrap();
        let x = cross(&c);
        assert_eq!((x.color.as_str(), x.gap, x.length, x.thickness), ("#FC0FC0", 4, 8, 3));
        assert!(x.outline.enabled && has_dot(&c));

        // legacy v4: 768p reference, gap 25, length 7, thickness 20 → at 768 unchanged.
        let (c, _) = decode("CSGO-G8oAC-RyvWc-Hi3CZ-voSwn-QJbfE", 768).unwrap();
        let x = cross(&c);
        assert_eq!((x.color.as_str(), x.gap, x.length, x.thickness), ("#7C3939", 25, 7, 20));
        assert!(x.outline.enabled && has_dot(&c));

        // legacy v1: green preset, length 5, thickness 0.5, gap 1, no outline (units, 480p basis).
        let (c, _) = decode("CSGO-LibdP-VCVEd-ESayK-rSivi-2UBtG", 480).unwrap();
        let x = cross(&c);
        assert_eq!((x.color.as_str(), x.length, x.thickness), ("#32FA32", 5, 1));
        assert!(!x.outline.enabled && !has_dot(&c));

        // legacy v1 with custom color, alpha, outline, dot and T-style.
        let (c, _) = decode("CSGO-Cn37R-YE7vo-pLCAL-aURmZ-z6zkG", 480).unwrap();
        let x = cross(&c);
        assert_eq!(x.color, "#AF51D589");
        assert!(x.t_style && x.outline.enabled && has_dot(&c));
        assert_eq!(x.outline.thickness, 2);
    }

    #[test]
    fn invalid_counter_strike_codes() {
        for code in [
            "CSGO-L9spZ-ihuov-cyhtE-kxbqa-FkBAA",
            "CSGO-eRqP8-AkrwM-3Wsqh-pKDTh-eAPtQ",
            "CSuZhVyU8icUwjTjopBSVi6ctEbq5GiOXJBRdC6LNLKaFL",
            "CSkSCHQKunGr2yPsJr2U3RTd8ycMA2pEs9jWnxK5KjMmWL",
            "CSoQhciXBHZUKAbfexZBeMWFKxkrXL5MLumDLj3JYW4snP",
            "CSGO-abc",
            "CS0000",
        ] {
            assert!(decode(code, 1080).is_err(), "{code} must be rejected");
        }
    }

    #[test]
    fn valorant_dot() {
        let (c, source) = decode("0;P;c;5;o;1;d;1;z;4;0b;0;1b;0", 1080).unwrap();
        assert_eq!(source, Source::Valorant);
        let [Layer::Dot(d)] = c.layers.as_slice() else { panic!("{:?}", c.layers) };
        assert_eq!((d.color.as_str(), d.size), ("#00FFFF", 4));
        assert_eq!(d.outline.color, "#000000");
    }

    #[test]
    fn valorant_separate_vertical_length() {
        let (c, _) = decode("0;P;h;0;0l;4;0v;9;0g;1;0o;2;1b;0", 1080).unwrap();
        let x = cross(&c);
        assert_eq!((x.length, x.vertical_length, x.vertical()), (4, Some(9), 9));
        // Without "g;1" the "v" value is ignored.
        let (c, _) = decode("0;P;h;0;0l;4;0v;9;1b;0", 1080).unwrap();
        assert_eq!(cross(&c).vertical_length, None);
    }

    #[test]
    fn valorant_lines_and_defaults() {
        // Inner lines only, custom color, outline off, sizes scaled from 1080p to 1440p.
        let (c, _) = decode("0;s;1;P;c;8;u;FF8800FF;h;0;0l;6;0o;3;0a;1;0f;0;1b;0;A;c;1", 1440).unwrap();
        let [Layer::Cross(x)] = c.layers.as_slice() else { panic!("{:?}", c.layers) };
        assert_eq!((x.color.as_str(), x.length, x.gap, x.thickness), ("#FF8800", 8, 4, 3));
        assert!(!x.outline.enabled);
        // Plain "0;P" uses Valorant's defaults: outer + inner lines, half-transparent outline.
        let (c, _) = decode("0;P", 1080).unwrap();
        assert_eq!(c.layers.len(), 2);
        assert_eq!(cross(&c).outline.color, "#00000080");
        assert!(decode("0;P;0b;0;1b;0", 1080).is_err());
    }

    #[test]
    fn unknown_text_is_rejected() {
        assert!(decode("hello", 1080).is_err());
        assert!(decode("", 1080).is_err());
    }
}
