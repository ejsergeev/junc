// SPDX-License-Identifier: GPL-3.0-or-later

//! Schema migrations (F22). Each step turns version N into N+1 on the
//! `toml_edit` document, so comments and layout survive. Steps are pure and
//! tested with fixture pairs in `tests/fixtures/configs/migrate/`.

use crate::diag::{Diagnostic, codes};
use crate::model::SCHEMA_VERSION;
use toml_edit::{DocumentMut, value};

type Step = fn(&mut DocumentMut);

/// `STEPS[n]` migrates version `n` to `n + 1`.
const STEPS: [Step; SCHEMA_VERSION as usize] = [v0_to_v1];

/// Version 0 is the pre-release format without a `schema` key; it is
/// otherwise identical to version 1.
fn v0_to_v1(doc: &mut DocumentMut) {
    doc.insert("schema", value(1));
    doc.sort_values_by(|k1, _, k2, _| (k1.get() != "schema").cmp(&(k2.get() != "schema")));
}

/// Brings `doc` to the current schema. Returns the version it had, or `None`
/// when it was already current.
pub fn migrate(doc: &mut DocumentMut) -> Result<Option<u32>, Diagnostic> {
    let from = match doc.get("schema") {
        None => 0,
        Some(item) => match item.as_integer() {
            Some(n) if (0..=i64::from(u32::MAX)).contains(&n) => n as u32,
            _ => {
                return Err(Diagnostic::error(
                    codes::SCHEMA_INVALID,
                    "`schema` must be a whole number",
                )
                .with_hint(format!("This build writes `schema = {SCHEMA_VERSION}`.")));
            }
        },
    };
    if from > SCHEMA_VERSION {
        return Err(Diagnostic::error(
            codes::SCHEMA_NEWER,
            format!("config schema {from} is newer than this JUNC understands ({SCHEMA_VERSION})"),
        )
        .with_hint("Update JUNC, or restore an older config; the last good config stays in use."));
    }
    if from == SCHEMA_VERSION {
        return Ok(None);
    }
    for step in &STEPS[from as usize..] {
        step(doc);
    }
    Ok(Some(from))
}
