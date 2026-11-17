//! Parses hotkey strings like "F8" or "Ctrl+Shift+X" into Win32 modifier flags and a virtual key.

pub const MOD_ALT: u32 = 0x1;
pub const MOD_CONTROL: u32 = 0x2;
pub const MOD_SHIFT: u32 = 0x4;
pub const MOD_WIN: u32 = 0x8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub modifiers: u32,
    pub vk: u32,
}

/// Empty input means "no hotkey".
pub fn parse(s: &str) -> Result<Option<Hotkey>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    let (key, mods) = parts.split_last().expect("split yields at least one part");
    let mut modifiers = 0;
    for m in mods {
        modifiers |= match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "strg" => MOD_CONTROL,
            "alt" => MOD_ALT,
            "shift" => MOD_SHIFT,
            "win" | "super" => MOD_WIN,
            other => return Err(format!("unbekannte Zusatztaste \"{other}\"")),
        };
    }
    let vk = key_code(key).ok_or_else(|| format!("unbekannte Taste \"{key}\""))?;
    Ok(Some(Hotkey { modifiers, vk }))
}

fn key_code(key: &str) -> Option<u32> {
    let k = key.to_ascii_lowercase();
    if k.len() == 1 {
        let c = k.as_bytes()[0];
        return match c {
            b'a'..=b'z' => Some((c - b'a') as u32 + 0x41),
            b'0'..=b'9' => Some((c - b'0') as u32 + 0x30),
            _ => None,
        };
    }
    if let Some(n) = k.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        return (1..=24).contains(&n).then_some(0x70 + n - 1);
    }
    for prefix in ["numpad", "num"] {
        if let Some(n) = k.strip_prefix(prefix).and_then(|n| n.parse::<u32>().ok()) {
            return (n <= 9).then_some(0x60 + n);
        }
    }
    Some(match k.as_str() {
        "insert" | "ins" | "einfg" => 0x2D,
        "delete" | "del" | "entf" => 0x2E,
        "home" | "pos1" => 0x24,
        "end" | "ende" => 0x23,
        "pageup" | "pgup" | "bildauf" => 0x21,
        "pagedown" | "pgdn" | "bildab" => 0x22,
        "pause" => 0x13,
        "scrolllock" | "rollen" => 0x91,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_keys() {
        assert_eq!(parse("F8").unwrap(), Some(Hotkey { modifiers: 0, vk: 0x77 }));
        assert_eq!(parse("ctrl + shift + x").unwrap(), Some(Hotkey { modifiers: MOD_CONTROL | MOD_SHIFT, vk: 0x58 }));
        assert_eq!(parse("Alt+5").unwrap(), Some(Hotkey { modifiers: MOD_ALT, vk: 0x35 }));
        assert_eq!(parse("Num4").unwrap(), Some(Hotkey { modifiers: 0, vk: 0x64 }));
        assert_eq!(parse("Insert").unwrap(), Some(Hotkey { modifiers: 0, vk: 0x2D }));
        assert_eq!(parse("").unwrap(), None);
    }

    #[test]
    fn rejects_unknown() {
        assert!(parse("F25").is_err());
        assert!(parse("Hyper+F1").is_err());
        assert!(parse("Ctrl+").is_err());
        assert!(parse("Ü").is_err());
    }
}
