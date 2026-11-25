// SPDX-License-Identifier: GPL-3.0-or-later

//! Semantic tokens from an Omarchy theme (docs/architecture.md §7.2, Spec
//! board §1): palette roles from `colors.toml`, control-state alphas,
//! spacing scale and font size from `shell.toml`, and the derived values
//! (mixes, contrast-corrected status inks) with the Spec's own formulas.
//! Built-in fallbacks: Tokyo Night (dark) and Flexoki Light (light).
//! No Slint here: the UI receives [`Tokens`] and only assigns them.

use std::path::Path;

/// An sRGB colour with alpha (0–255).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// `#rrggbb` or `#rrggbbaa`.
    pub fn parse(s: &str) -> Option<Self> {
        let h = s.trim().strip_prefix('#')?;
        let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
        match h.len() {
            6 => Some(Self::rgb(byte(0)?, byte(2)?, byte(4)?)),
            8 => Some(Self {
                a: byte(6)?,
                ..Self::rgb(byte(0)?, byte(2)?, byte(4)?)
            }),
            _ => None,
        }
    }

    /// `#rrggbb` (alpha left out when opaque).
    pub fn hex(self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }

    /// The same colour at `alpha` (0–1).
    pub fn alpha(self, alpha: f64) -> Self {
        Self {
            a: to_byte(alpha.clamp(0.0, 1.0) * 255.0),
            ..self
        }
    }

    /// `self` moved toward `other` by `t` (0–1), per channel, rounded: the
    /// Spec's `mix`.
    pub fn mix(self, other: Self, t: f64) -> Self {
        let m = |x: u8, y: u8| to_byte(f64::from(x) * (1.0 - t) + f64::from(y) * t);
        Self::rgb(m(self.r, other.r), m(self.g, other.g), m(self.b, other.b))
    }

    /// WCAG relative luminance.
    fn luminance(self) -> f64 {
        let ch = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(self.r) + 0.7152 * ch(self.g) + 0.0722 * ch(self.b)
    }

    /// WCAG contrast ratio.
    pub fn contrast(self, other: Self) -> f64 {
        let (x, y) = (self.luminance(), other.luminance());
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }
}

fn to_byte(v: f64) -> u8 {
    // Clamped first, so the cast cannot truncate.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let b = v.round().clamp(0.0, 255.0) as u8;
    b
}

/// The palette roles JUNC uses (Omarchy `colors.toml` names).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub dark: bool,
    pub background: Color,
    pub dark_background: Color,
    pub darker_background: Color,
    pub lighter_background: Color,
    pub foreground: Color,
    pub dark_foreground: Color,
    pub bright_foreground: Color,
    pub accent: Color,
    pub muted: Color,
    pub selection: Color,
    pub red: Color,
    pub yellow: Color,
    pub green: Color,
}

/// The `[controls]` alphas and other values from `shell.toml`.
#[derive(Debug, Clone, PartialEq)]
pub struct Shell {
    pub normal_fill: f64,
    pub normal_border: f64,
    pub hover_fill: f64,
    pub hover_border: f64,
    pub focus_fill: f64,
    pub focus_border: f64,
    pub selected_fill: f64,
    pub pressed_fill: f64,
    pub selection_fill: f64,
    /// `[spacing] scale`.
    pub scale: f64,
    /// `[font] base-size`.
    pub font_size: f64,
}

impl Default for Shell {
    /// Omarchy's defaults (Spec: 4 / 8 / 8 / 18 / 22 / 35 %).
    fn default() -> Self {
        Self {
            normal_fill: 0.04,
            normal_border: 0.4,
            hover_fill: 0.08,
            hover_border: 0.25,
            focus_fill: 0.08,
            focus_border: 0.25,
            selected_fill: 0.18,
            pressed_fill: 0.22,
            selection_fill: 0.35,
            scale: 1.0,
            font_size: 12.0,
        }
    }
}

/// Every semantic token the components use (Spec §1).
#[derive(Debug, Clone, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    pub surface_desktop: Color,
    pub surface_window: Color,
    pub surface_sidebar: Color,
    pub surface_card: Color,
    pub surface_card_header: Color,
    pub text_default: Color,
    pub text_strong: Color,
    pub text_subtle: Color,
    pub mark_dim: Color,
    pub line_divider: Color,
    pub accent: Color,
    pub selection: Color,
    pub meter_safe: Color,
    pub meter_hot: Color,
    pub meter_clip: Color,
    /// Status words on a card, darkened toward the foreground until 4.5:1.
    pub status_urgent: Color,
    pub status_ok: Color,
    pub status_warn: Color,
    pub control_normal_fill: Color,
    pub control_normal_border: Color,
    pub control_hover_fill: Color,
    pub control_hover_border: Color,
    pub control_focus_fill: Color,
    pub control_focus_border: Color,
    pub control_selected_fill: Color,
    pub control_pressed_fill: Color,
    pub text_selection_fill: Color,
    pub track: Color,
    pub halo: Color,
    /// Spacing multiplier (`[spacing] scale`).
    pub scale: f64,
    /// Body text size in px (`[font] base-size`).
    pub font_size: f64,
}

/// The Spec's `ink`: `col` mixed toward the foreground in 10 % steps until
/// it reaches 4.5:1 on the card.
fn ink(p: &Palette, col: Color) -> Color {
    let mut t = 0.0;
    let mut out = col;
    while out.contrast(p.lighter_background) < 4.5 && t < 1.0 {
        t += 0.1;
        out = col.mix(p.foreground, t);
    }
    out
}

impl Tokens {
    /// Derives every token (Spec §1 formulas).
    pub fn derive(p: &Palette, s: &Shell) -> Self {
        let fg = p.foreground;
        Self {
            dark: p.dark,
            surface_desktop: p.darker_background,
            surface_window: p.background,
            surface_sidebar: p.dark_background,
            surface_card: p.lighter_background,
            surface_card_header: p.lighter_background.mix(fg, 0.04),
            text_default: fg,
            text_strong: p.bright_foreground,
            text_subtle: fg.mix(p.background, 0.2),
            mark_dim: p.dark_foreground,
            line_divider: p.muted,
            accent: p.accent,
            selection: p.selection,
            meter_safe: p.green,
            meter_hot: p.yellow,
            meter_clip: p.red,
            status_urgent: ink(p, p.red),
            status_ok: ink(p, p.green),
            status_warn: ink(p, p.yellow),
            control_normal_fill: fg.alpha(s.normal_fill),
            control_normal_border: fg.alpha(s.normal_border),
            control_hover_fill: fg.alpha(s.hover_fill),
            control_hover_border: fg.alpha(s.hover_border),
            control_focus_fill: fg.alpha(s.focus_fill),
            control_focus_border: fg.alpha(s.focus_border),
            control_selected_fill: fg.alpha(s.selected_fill),
            control_pressed_fill: fg.alpha(s.pressed_fill),
            text_selection_fill: fg.alpha(s.selection_fill),
            track: fg.alpha(0.1),
            halo: p.accent.alpha(0.25),
            scale: s.scale,
            font_size: s.font_size,
        }
    }
}

/// Tokyo Night, the built-in dark theme (Spec's reference values).
pub fn tokyo_night() -> Palette {
    let c = |s| Color::parse(s).unwrap_or(Color::rgb(0, 0, 0));
    Palette {
        dark: true,
        background: c("#1a1b26"),
        dark_background: c("#13141c"),
        darker_background: c("#0e0e14"),
        lighter_background: c("#24283b"),
        foreground: c("#a9b1d6"),
        dark_foreground: c("#565f89"),
        bright_foreground: c("#c0caf5"),
        accent: c("#7aa2f7"),
        muted: c("#414868"),
        selection: c("#292e42"),
        red: c("#f7768e"),
        yellow: c("#e0af68"),
        green: c("#9ece6a"),
    }
}

/// Flexoki Light, the built-in light theme (Spec's reference values).
pub fn flexoki_light() -> Palette {
    let c = |s| Color::parse(s).unwrap_or(Color::rgb(255, 255, 255));
    Palette {
        dark: false,
        background: c("#fffcf0"),
        dark_background: c("#f2efe4"),
        darker_background: c("#e5e2d8"),
        lighter_background: c("#e6e4d9"),
        foreground: c("#100f0f"),
        dark_foreground: c("#878580"),
        bright_foreground: c("#100f0f"),
        accent: c("#205ea6"),
        muted: c("#b7b5ac"),
        selection: c("#cecdc3"),
        red: c("#d14d41"),
        yellow: c("#d0a215"),
        green: c("#879a39"),
    }
}

/// Reads `colors.toml`. A role the theme lacks takes the nearest one it
/// has (Omarchy's generated themes always have them all).
pub fn palette_from(colors: &str) -> Result<Palette, String> {
    let t: toml::Table = colors.parse().map_err(|e| format!("colors.toml: {e}"))?;
    let get = |k: &str| {
        t.get(k)
            .and_then(toml::Value::as_str)
            .and_then(Color::parse)
    };
    let need = |k: &str| get(k).ok_or_else(|| format!("colors.toml: no {k}"));
    let background = need("background")?;
    let foreground = need("foreground")?;
    let accent = get("accent")
        .or(get("blue"))
        .ok_or("colors.toml: no accent")?;
    let dark = match t.get("mode").and_then(toml::Value::as_str) {
        Some(m) => m != "light",
        None => background.contrast(Color::rgb(0, 0, 0)) < foreground.contrast(Color::rgb(0, 0, 0)),
    };
    let or = |k: &str, f: Color| get(k).unwrap_or(f);
    Ok(Palette {
        dark,
        background,
        dark_background: or("dark_background", background),
        darker_background: or("darker_background", or("dark_background", background)),
        lighter_background: or("lighter_background", background),
        foreground,
        dark_foreground: or("dark_foreground", foreground.mix(background, 0.4)),
        bright_foreground: or("bright_foreground", foreground),
        accent,
        muted: or("muted", foreground.mix(background, 0.6)),
        selection: or("selection", background.mix(foreground, 0.15)),
        red: need("red")?,
        yellow: need("yellow")?,
        green: need("green")?,
    })
}

/// Reads `shell.toml`; anything missing keeps Omarchy's default.
pub fn shell_from(text: &str) -> Shell {
    let mut s = Shell::default();
    let Ok(t) = text.parse::<toml::Table>() else {
        return s;
    };
    let num = |section: &str, key: &str| {
        let v = t.get(section)?.get(key)?;
        v.as_float().or_else(|| v.as_integer().map(|i| i as f64))
    };
    let set = |field: &mut f64, section: &str, key: &str| {
        if let Some(v) = num(section, key) {
            *field = v;
        }
    };
    set(&mut s.normal_fill, "controls", "normal-fill-alpha");
    set(&mut s.normal_border, "controls", "normal-border-alpha");
    set(&mut s.hover_fill, "controls", "hover-cursor-fill-alpha");
    set(&mut s.hover_border, "controls", "hover-cursor-border-alpha");
    set(&mut s.focus_fill, "controls", "focus-fill-alpha");
    set(&mut s.focus_border, "controls", "focus-border-alpha");
    set(&mut s.selected_fill, "controls", "selected-fill-alpha");
    set(&mut s.pressed_fill, "controls", "pressed-fill-alpha");
    set(&mut s.selection_fill, "controls", "selection-fill-alpha");
    set(&mut s.scale, "spacing", "scale");
    set(&mut s.font_size, "font", "base-size");
    s
}

/// Where Omarchy keeps the current theme: `$OMARCHY_THEME_DIR` (tests),
/// else `~/.local/state/omarchy/current/theme`.
pub fn omarchy_dir() -> Option<std::path::PathBuf> {
    if let Some(d) = std::env::var_os("OMARCHY_THEME_DIR").filter(|v| !v.is_empty()) {
        return Some(d.into());
    }
    let state = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/state")))?;
    Some(state.join("omarchy/current/theme"))
}

/// Tokens of the Omarchy theme in `dir`.
pub fn load(dir: &Path) -> Result<Tokens, String> {
    let colors = std::fs::read_to_string(dir.join("colors.toml"))
        .map_err(|e| format!("{}: {e}", dir.join("colors.toml").display()))?;
    let shell = std::fs::read_to_string(dir.join("shell.toml")).unwrap_or_default();
    Ok(Tokens::derive(&palette_from(&colors)?, &shell_from(&shell)))
}

/// The built-in theme: Flexoki Light when the desktop prefers light (the
/// portal's `color-scheme`), else Tokyo Night.
pub fn fallback(prefer_light: bool) -> Tokens {
    let p = if prefer_light {
        flexoki_light()
    } else {
        tokyo_night()
    };
    Tokens::derive(&p, &Shell::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_parse_mix_and_measure_like_the_spec() {
        let bg = Color::parse("#1a1b26").expect("hex");
        let fg = Color::parse("#a9b1d6").expect("hex");
        assert_eq!(bg.hex(), "#1a1b26");
        assert_eq!(Color::parse("#a9b1d680").expect("hex").a, 0x80);
        assert_eq!(Color::parse("1a1b26"), None);
        // Spec: text.subtle on Tokyo Night is #8c93b3 (mix fg→bg 20 %).
        assert_eq!(fg.mix(bg, 0.2).hex(), "#8c93b3");
        // dark_foreground fails WCAG AA for text on the background (the
        // Spec's reason for text.subtle's own recipe).
        let dfg = Color::parse("#565f89").expect("hex");
        assert!(dfg.contrast(bg) < 4.5, "{}", dfg.contrast(bg));
        assert!(fg.mix(bg, 0.2).contrast(bg) >= 4.5);
        assert_eq!(fg.alpha(0.04).a, 10);
    }

    /// The values the Spec board shows for both built-in themes.
    #[test]
    fn derived_tokens_match_the_spec_board() {
        let tk = Tokens::derive(&tokyo_night(), &Shell::default());
        let fx = Tokens::derive(&flexoki_light(), &Shell::default());
        let pairs = [
            (tk.surface_card_header, "#292d41"),
            (tk.text_subtle, "#8c93b3"),
            (tk.status_urgent, "#f7768e"),
            (fx.surface_card_header, "#dddbd1"),
            (fx.text_subtle, "#403e3c"),
        ];
        for (got, want) in pairs {
            assert_eq!(got.hex(), want);
        }
        // Status inks reach 4.5:1 on the card.
        for t in [&tk, &fx] {
            for ink in [t.status_urgent, t.status_ok, t.status_warn] {
                assert!(ink.contrast(t.surface_card) >= 4.5, "{}", ink.hex());
            }
        }
        assert!(tk.dark && !fx.dark);
    }

    #[test]
    fn an_omarchy_theme_reads_with_defaults_for_what_it_lacks() {
        let colors = "mode = \"dark\"\naccent = \"#7aa2f7\"\nbackground = \"#1a1b26\"\nforeground = \"#a9b1d6\"\nred = \"#f7768e\"\nyellow = \"#e0af68\"\ngreen = \"#9ece6a\"\n";
        let p = palette_from(colors).expect("palette");
        assert_eq!(p.bright_foreground, p.foreground);
        assert_eq!(p.dark_background, p.background);
        let s = shell_from(
            "[controls]\nnormal-fill-alpha = 0.1\n[font]\nbase-size = 14\n[spacing]\nscale = 1.25\n",
        );
        assert_eq!((s.normal_fill, s.font_size, s.scale), (0.1, 14.0, 1.25));
        assert_eq!(s.hover_fill, 0.08, "default kept");
        assert_eq!(shell_from("not toml ["), Shell::default());
        assert!(palette_from("background = \"#000000\"").is_err());
    }
}
