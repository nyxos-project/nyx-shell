//! Key combinations as Hyprland writes them (`SUPER + SHIFT + D`).

/// Hyprland's modifier bits, as `hyprctl -j binds` reports them in `modmask`.
const MODS: &[(&str, u32)] = &[
    ("SHIFT", 1),
    ("CAPS", 2),
    ("CTRL", 4),
    ("CONTROL", 4),
    ("ALT", 8),
    ("MOD2", 16),
    ("MOD3", 32),
    ("SUPER", 64),
    ("WIN", 64),
    ("LOGO", 64),
    ("MOD4", 64),
    ("MOD5", 128),
];

/// Order modifiers are written in.
const ORDER: &[(&str, u32)] = &[("SUPER", 64), ("CTRL", 4), ("ALT", 8), ("SHIFT", 1), ("CAPS", 2), ("MOD2", 16), ("MOD3", 32), ("MOD5", 128)];

/// Descriptions of the binds Shell Core writes start with this, so `hyprctl -j binds`
/// shows which ones are its own.
pub const DESCRIPTION_PREFIX: &str = "Nyx Shell: ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    pub mods: u32,
    /// Upper case for letters and names (`D`, `SPACE`, `F1`).
    pub key: String,
}

impl Shortcut {
    pub fn parse(text: &str) -> Result<Shortcut, String> {
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let (key, mods) = parts.split_last().ok_or("empty shortcut")?;
        if key.is_empty() || key.len() > 32 || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Err(format!("{text:?}: the key must be a letter, digit or key name"));
        }
        let mut mask = 0;
        for m in mods {
            let upper = m.to_ascii_uppercase();
            let bit = MODS.iter().find(|(n, _)| *n == upper).map(|(_, b)| *b).ok_or_else(|| format!("{text:?}: unknown modifier {m:?}"))?;
            mask |= bit;
        }
        Ok(Shortcut { mods: mask, key: key.to_ascii_uppercase() })
    }

    /// `SUPER + SHIFT + D`.
    pub fn canonical(&self) -> String {
        let mut out: Vec<&str> = ORDER.iter().filter(|(_, b)| self.mods & b != 0).map(|(n, _)| *n).collect();
        out.push(&self.key);
        out.join(" + ")
    }

    /// Whether a bind from `hyprctl -j binds` uses the same keys.
    pub fn matches(&self, modmask: u32, key: &str) -> bool {
        self.mods == modmask && self.key.eq_ignore_ascii_case(key)
    }

    /// The Lua line for `nyx-shell.lua`. `action` must be a valid id (checked here), so
    /// nothing but id characters reaches the quoted strings.
    pub fn lua_bind(&self, action: &str) -> Result<String, String> {
        if !crate::valid_id(action) {
            return Err(format!("invalid action id {action:?}"));
        }
        Ok(format!(
            "hl.bind(\"{keys}\", hl.dsp.exec_cmd(\"nyx-shellctl invoke {action}\"), {{ description = \"{DESCRIPTION_PREFIX}{action}\" }})",
            keys = self.canonical()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_writes() {
        let s = Shortcut::parse("super + shift + d").unwrap();
        assert_eq!(s.mods, 65);
        assert_eq!(s.canonical(), "SUPER + SHIFT + D");
        assert!(s.matches(65, "d"));
        assert!(!s.matches(64, "D"));
        assert_eq!(Shortcut::parse("SPACE").unwrap().mods, 0);
        assert_eq!(Shortcut::parse("CTRL + ALT + F1").unwrap().canonical(), "CTRL + ALT + F1");
    }

    #[test]
    fn refuses_nonsense() {
        assert!(Shortcut::parse("").is_err());
        assert!(Shortcut::parse("SUPER + ").is_err());
        assert!(Shortcut::parse("HYPER + D").is_err());
        assert!(Shortcut::parse("SUPER + \"); os.exit(").is_err());
    }

    #[test]
    fn lua_line() {
        let s = Shortcut::parse("SUPER + D").unwrap();
        assert_eq!(
            s.lua_bind("shell.control-center.toggle").unwrap(),
            "hl.bind(\"SUPER + D\", hl.dsp.exec_cmd(\"nyx-shellctl invoke shell.control-center.toggle\"), { description = \"Nyx Shell: shell.control-center.toggle\" })"
        );
        assert!(s.lua_bind("a\"b").is_err());
    }
}
