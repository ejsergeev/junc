// SPDX-License-Identifier: GPL-3.0-or-later

//! The published JSON Schema of the config and preset format (`junc schema`,
//! `schema/junc-config-v1.json`).

use crate::model::Config;

/// JSON Schema of `junctions.toml` for the current schema version.
pub fn json_schema() -> String {
    let schema = schemars::schema_for!(Config);
    let mut text = serde_json::to_string_pretty(&schema).unwrap_or_default();
    text.push('\n');
    text
}
