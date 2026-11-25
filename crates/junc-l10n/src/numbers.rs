// SPDX-License-Identifier: GPL-3.0-or-later

//! Numbers in the language's format (F14), with ICU4X: separators,
//! grouping and the numbering system Omarchy's registry names (`۰` digits
//! for Persian, `০` for Bengali). Messages take the formatted text as a
//! placeholder; `{n}` only picks the plural form (validation finding: Slint
//! and gettext print `{n}` with ASCII digits).

use crate::registry;
use icu_decimal::DecimalFormatter;
use icu_decimal::input::Decimal;
use icu_locale_core::{Locale, locale};

/// The Unicode numbering system whose zero is `zero`.
fn numbering_system(zero: char) -> Option<&'static str> {
    Some(match zero {
        '0' => return None,
        '٠' => "arab",
        '۰' => "arabext",
        '০' => "beng",
        '०' => "deva",
        '௦' => "tamldec",
        '๐' => "thai",
        _ => return None,
    })
}

/// The ICU locale for a registry language (`fa-IR-u-nu-arabext`).
fn locale(code: &str) -> Option<Locale> {
    let lang = registry::find(code)?;
    let tag = match numbering_system(lang.zero_digit) {
        Some(nu) => format!("{}-u-nu-{nu}", lang.format_locale),
        None => lang.format_locale.clone(),
    };
    tag.parse().ok()
}

/// `value` with exactly `decimals` digits after the separator, in the
/// format of language `code` (English for unknown codes and `en_XA`).
pub fn format_in(code: &str, value: f64, decimals: u8) -> String {
    let fallback = || format!("{value:.*}", usize::from(decimals));
    // Scaled to a whole number, then shifted back: no float parsing needed.
    let scaled = (value * 10f64.powi(i32::from(decimals))).round();
    if !scaled.is_finite() || scaled.abs() > 9.0e15 {
        return fallback();
    }
    // Checked above: whole, finite and well inside i64.
    #[allow(clippy::cast_possible_truncation)]
    let mut d = Decimal::from(scaled as i64);
    let position = -i16::from(decimals);
    d.multiply_pow10(position);
    d.absolute.pad_end(position);
    let locale = locale(code).unwrap_or(locale!("en-US"));
    match DecimalFormatter::try_new((&locale).into(), Default::default()) {
        Ok(f) => f.format(&d).to_string(),
        Err(_) => fallback(),
    }
}

/// [`format_in`] for the language in use.
pub fn format(value: f64, decimals: u8) -> String {
    format_in(&crate::language(), value, decimals)
}

/// A whole number in the language in use.
pub fn count(n: u64) -> String {
    // u64 → f64 is exact up to 2^53, far beyond any count JUNC shows.
    #[allow(clippy::cast_precision_loss)]
    format(n as f64, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_follow_the_language() {
        assert_eq!(format_in("en", 1234.5, 1), "1,234.5");
        assert_eq!(format_in("de", 1234.5, 1), "1.234,5");
        assert_eq!(format_in("da", 21.33, 1), "21,3");
        assert_eq!(format_in("fr", 48000.0, 0), "48\u{202f}000");
        assert_eq!(format_in("fa", 21.3, 1), "۲۱٫۳");
        assert_eq!(format_in("bn", 1024.0, 0), "১,০২৪");
        assert_eq!(format_in("en", 2.0, 2), "2.00");
        assert_eq!(format_in("en_XA", 2.5, 1), "2.5");
        assert_eq!(format_in("tlh", 2.5, 1), "2.5");
    }
}
