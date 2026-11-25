// SPDX-License-Identifier: GPL-3.0-or-later

//! Actionable diagnostics: what is wrong, where, and how to fix it (F22, F25).
//! Codes are stable strings; agents and the GUI match on them.

use serde::Serialize;
use std::fmt;

/// How bad a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The config is rejected.
    Error,
    /// The config is accepted but probably not what was meant.
    Warning,
}

/// A position in a config file. Lines and columns are 1-based; columns count
/// characters, not bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Location {
    /// File as given by the caller (e.g. `~/.config/junc/junctions.toml`).
    pub file: String,
    /// 1-based line.
    pub line: usize,
    /// 1-based column.
    pub column: usize,
}

/// One finding about a config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    /// Stable code, e.g. `E_CONFIG_UNKNOWN_KEY`.
    pub code: &'static str,
    /// Error or warning.
    pub severity: Severity,
    /// What is wrong, in one sentence.
    pub message: String,
    /// Where, when the problem comes from a file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    /// How to fix it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl Diagnostic {
    /// A new error without location or hint.
    pub fn error(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Error,
            message: message.into(),
            location: None,
            hint: None,
        }
    }

    /// A new warning without location or hint.
    pub fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            ..Self::error(code, message)
        }
    }

    /// Adds a hint.
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Adds a location.
    #[must_use]
    pub fn at(mut self, location: Option<Location>) -> Self {
        self.location = location;
        self
    }

    /// True for errors.
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(l) = &self.location {
            write!(f, "{}:{}:{}: ", l.file, l.line, l.column)?;
        }
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{sev}[{}]: {}", self.code, self.message)?;
        if let Some(h) = &self.hint {
            write!(f, "\n  hint: {h}")?;
        }
        Ok(())
    }
}

/// Maps byte offsets in a source text to 1-based line and column.
#[derive(Debug)]
pub struct SourceMap<'a> {
    file: &'a str,
    src: &'a str,
}

impl<'a> SourceMap<'a> {
    /// A map for `src`, reported as `file`.
    pub fn new(file: &'a str, src: &'a str) -> Self {
        Self { file, src }
    }

    /// Location of a byte offset (clamped to the text).
    pub fn locate(&self, offset: usize) -> Location {
        let offset = offset.min(self.src.len());
        let before = &self.src[..self.src.floor_char_boundary(offset)];
        let line = before.matches('\n').count() + 1;
        let line_start = before.rfind('\n').map_or(0, |i| i + 1);
        let column = before[line_start..].chars().count() + 1;
        Location {
            file: self.file.to_owned(),
            line,
            column,
        }
    }
}

/// Codes used by the config loader. Documented in the skill's reference.
pub mod codes {
    /// The file is not valid TOML.
    pub const TOML_SYNTAX: &str = "E_CONFIG_SYNTAX";
    /// The `schema` key is newer than this build understands.
    pub const SCHEMA_NEWER: &str = "E_CONFIG_SCHEMA_NEWER";
    /// The `schema` key is not a known version.
    pub const SCHEMA_INVALID: &str = "E_CONFIG_SCHEMA_INVALID";
    /// A key that the format does not have.
    pub const UNKNOWN_KEY: &str = "E_CONFIG_UNKNOWN_KEY";
    /// A required key is missing.
    pub const MISSING_KEY: &str = "E_CONFIG_MISSING_KEY";
    /// A value has the wrong type or shape.
    pub const INVALID_VALUE: &str = "E_CONFIG_INVALID_VALUE";
    /// An id is not `[a-z0-9-]+`.
    pub const ID_FORMAT: &str = "E_CONFIG_ID_FORMAT";
    /// Two items share an id.
    pub const ID_DUPLICATE: &str = "E_CONFIG_ID_DUPLICATE";
    /// A number is out of its allowed range.
    pub const RANGE: &str = "E_CONFIG_RANGE";
    /// A Link refers to a channel that does not exist.
    pub const LINK_CHANNEL: &str = "E_CONFIG_LINK_CHANNEL";
    /// More than one Junction claims the same default role.
    pub const DEFAULT_DUPLICATE: &str = "E_CONFIG_DEFAULT_DUPLICATE";
    /// A match has no keys, or keys of several kinds.
    pub const MATCH_KIND: &str = "E_CONFIG_MATCH_KIND";
    /// A nested Junction id does not exist.
    pub const JUNCTION_UNKNOWN: &str = "E_CONFIG_JUNCTION_UNKNOWN";
    /// Nesting would form a loop.
    pub const CYCLE: &str = "E_CONFIG_CYCLE";
    /// A Cable is not shaped like a Cable.
    pub const CABLE: &str = "E_CONFIG_CABLE";
    /// A template slot in a config that is not a preset.
    pub const SLOT_OUTSIDE_PRESET: &str = "E_CONFIG_SLOT_OUTSIDE_PRESET";
    /// Many Inputs on one Junction (warning).
    pub const MANY_INPUTS: &str = "W_CONFIG_MANY_INPUTS";
}
