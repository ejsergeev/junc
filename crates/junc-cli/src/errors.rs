// SPDX-License-Identifier: GPL-3.0-or-later

//! Every error code `junc` can report, its exit code (architecture §5.3)
//! and what to do about it. The skill's reference is generated from this
//! table; a test keeps it complete.

/// One error code.
#[derive(Debug, Clone, Copy)]
pub struct Code {
    /// Stable code, as in `--json` output.
    pub code: &'static str,
    /// Process exit code.
    pub exit: u8,
    /// What it means and what to do.
    pub meaning: &'static str,
}

const fn c(code: &'static str, exit: u8, meaning: &'static str) -> Code {
    Code {
        code,
        exit,
        meaning,
    }
}

/// All codes, grouped as the reference shows them.
pub const CODES: &[Code] = &[
    c(
        "E_USAGE",
        2,
        "Wrong command or arguments. See `junc --help` or the command's help.",
    ),
    c(
        "E_CONFIG_SYNTAX",
        3,
        "The file is not valid TOML. Fix it at `location`.",
    ),
    c(
        "E_CONFIG_SCHEMA_NEWER",
        3,
        "The file is for a newer JUNC. Update JUNC.",
    ),
    c(
        "E_CONFIG_SCHEMA_INVALID",
        3,
        "`schema` is not a known version. Use `schema = 1`.",
    ),
    c(
        "E_CONFIG_UNKNOWN_KEY",
        3,
        "A key the format does not have. The hint names the closest one.",
    ),
    c(
        "E_CONFIG_MISSING_KEY",
        3,
        "A required key is missing (for example `id` or `name`).",
    ),
    c(
        "E_CONFIG_INVALID_VALUE",
        3,
        "A value has the wrong type or shape.",
    ),
    c(
        "E_CONFIG_ID_FORMAT",
        3,
        "An id must be lowercase letters, digits and `-`.",
    ),
    c(
        "E_CONFIG_ID_DUPLICATE",
        3,
        "Two items share an id. Rename one.",
    ),
    c(
        "E_CONFIG_RANGE",
        3,
        "A number is out of range (volume 0–150, Bus 1–64 channels, rate 44100–192000).",
    ),
    c(
        "E_CONFIG_LINK_CHANNEL",
        3,
        "A link names a channel that does not exist.",
    ),
    c(
        "E_CONFIG_DEFAULT_DUPLICATE",
        3,
        "Two Junctions claim the same default role. Keep one.",
    ),
    c(
        "E_CONFIG_MATCH_KIND",
        3,
        "A `match` is empty or mixes kinds (app keys, `device`, `monitor`, `junction`).",
    ),
    c(
        "E_CONFIG_JUNCTION_UNKNOWN",
        3,
        "A nested Junction id does not exist.",
    ),
    c(
        "E_CONFIG_CYCLE",
        3,
        "Nesting would make audio feed back. Remove one Junction Input of the loop.",
    ),
    c(
        "E_CONFIG_CABLE",
        3,
        "A Cable needs exactly one app Input and `[junction.cable] to = { … }`.",
    ),
    c(
        "E_CONFIG_SLOT_OUTSIDE_PRESET",
        3,
        "Template slots are only for presets; use real values in the config.",
    ),
    c(
        "E_PRESET_SLOT",
        3,
        "A preset's slot is undeclared, declared twice, unused, or of a kind that does not fit where it is used.",
    ),
    c(
        "E_PRESET_FILL",
        2,
        "A template needs a value for each of its slots (`--slot ID=VALUE`), and only for those. The hint lists them.",
    ),
    c(
        "W_CONFIG_MANY_INPUTS",
        0,
        "Warning: many Inputs on one Junction. Accepted.",
    ),
    c(
        "E_NOT_FOUND",
        4,
        "No such Junction, Input, Listen output, link or unit. The hint suggests the closest name.",
    ),
    c(
        "E_CONFIG_CONFLICT",
        5,
        "The config changed since the revision you read. Read it again and redo the change.",
    ),
    c(
        "E_DAEMON_UNAVAILABLE",
        6,
        "juncd is not running. `systemctl --user start juncd`.",
    ),
    c(
        "E_DBUS",
        6,
        "No session bus, or the call failed on the bus.",
    ),
    c(
        "E_PIPEWIRE_UNAVAILABLE",
        7,
        "PipeWire is not running; plans need the live graph. `systemctl --user start pipewire wireplumber`.",
    ),
    c(
        "E_NEEDS_CONFIRMATION",
        8,
        "A destructive command without `--yes`. Ask the user, then pass `--yes`.",
    ),
    c(
        "E_WRITE_FAILED",
        1,
        "juncd could not write the config file. Check permissions of the config directory.",
    ),
    c(
        "E_EDITOR",
        1,
        "The editor of `junc config edit` failed. Set $EDITOR.",
    ),
    c(
        "E_NO_SYSTEMD",
        1,
        "`junc service` needs systemd. Start juncd from the session's autostart instead.",
    ),
    c(
        "E_SERVICE",
        1,
        "systemctl failed. See `systemctl --user status juncd`.",
    ),
    c(
        "E_SKILL",
        1,
        "The skill could not be linked; the message names the folder.",
    ),
    c(
        "E_OMARCHY",
        1,
        "JUNC could not be put into (or taken out of) the Omarchy bar and menu; the message names the file.",
    ),
    c(
        "E_INTERNAL",
        1,
        "A bug or a version mismatch between junc and juncd. Report it with `junc doctor`.",
    ),
];

/// Exit code for an error code (1 for anything unknown).
pub fn exit_code(code: &str) -> u8 {
    CODES.iter().find(|c| c.code == code).map_or(1, |c| c.exit)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every code literal in the sources is in the table.
    #[test]
    fn the_table_lists_every_code_in_the_sources() {
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut missing = Vec::new();
        for krate in std::fs::read_dir(&crates).expect("crates").flatten() {
            let src = krate.path().join("src");
            let Ok(files) = std::fs::read_dir(&src) else {
                continue;
            };
            for f in files
                .flatten()
                .filter(|f| f.path().extension().is_some_and(|e| e == "rs"))
            {
                let text = std::fs::read_to_string(f.path()).unwrap_or_default();
                for part in text.split('"').skip(1).step_by(2) {
                    let code_like = (part.starts_with("E_") || part.starts_with("W_"))
                        && part.len() > 3
                        && !part.ends_with('_')
                        && part.chars().all(|c| c.is_ascii_uppercase() || c == '_');
                    if code_like && CODES.iter().all(|c| c.code != part) {
                        missing.push(format!("{part} ({})", f.path().display()));
                    }
                }
            }
        }
        assert!(
            missing.is_empty(),
            "codes missing from errors::CODES: {missing:?}"
        );
    }

    #[test]
    fn exit_codes_follow_the_architecture() {
        assert_eq!(exit_code("E_USAGE"), 2);
        assert_eq!(exit_code("E_CONFIG_RANGE"), 3);
        assert_eq!(exit_code("E_NOT_FOUND"), 4);
        assert_eq!(exit_code("E_CONFIG_CONFLICT"), 5);
        assert_eq!(exit_code("E_DAEMON_UNAVAILABLE"), 6);
        assert_eq!(exit_code("E_PIPEWIRE_UNAVAILABLE"), 7);
        assert_eq!(exit_code("E_NEEDS_CONFIRMATION"), 8);
        assert_eq!(exit_code("E_SOMETHING_NEW"), 1);
    }
}
