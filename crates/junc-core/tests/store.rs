// SPDX-License-Identifier: GPL-3.0-or-later

//! Config store: hand edits, last good config, conflicts, and writes that
//! keep comments (F22).

use junc_core::model::Config;
use junc_core::store::{CONFLICT, Reload, Store};

const FILE: &str = r#"# My audio setup. Hand-written, please keep my notes.
schema = 1

# The one I use for meetings.
[[junction]]
id = "zoom"   # do not rename: scripts use it
name = "Zoom Mic"

[[junction.input]]
id = "spotify"
match = { binary = "spotify" }
volume = 70   # quieter than the mic

[[junction.input]]
id = "mic"
match = { device = "alsa_input.usb-mic" }

# Streaming.
[[junction]]
id = "obs"
name = "OBS Mix"
"#;

fn store() -> Store {
    Store::open(FILE, "junctions.toml").expect("valid")
}

fn write(s: &mut Store, f: impl FnOnce(&mut Config)) -> String {
    let mut c = s.config().clone();
    f(&mut c);
    let rev = s.revision();
    let (text, new_rev) = s.update(Some(rev), c.clone()).expect("update");
    assert_eq!(new_rev, rev + 1);
    assert_eq!(s.config(), &c);
    text
}

fn keeps_all_comments(text: &str) {
    for note in [
        "# My audio setup. Hand-written, please keep my notes.",
        "# The one I use for meetings.",
        "# do not rename: scripts use it",
        "# Streaming.",
    ] {
        assert!(text.contains(note), "lost {note:?} in:\n{text}");
    }
}

#[test]
fn changing_one_value_touches_only_that_value() {
    let mut s = store();
    let text = write(&mut s, |c| c.junction[0].input[0].volume = 55);
    keeps_all_comments(&text);
    assert!(
        text.contains("volume = 55   # quieter than the mic"),
        "value comment kept:\n{text}"
    );
    // Defaults the user never wrote stay unwritten.
    assert!(!text.contains("enabled"), "{text}");
    assert_eq!(text.replace("volume = 55", "volume = 70"), FILE);
}

#[test]
fn adding_and_removing_entries_keeps_the_rest() {
    let mut s = store();
    let text = write(&mut s, |c| {
        let mut i = c.junction[0].input[1].clone();
        i.id = "soundux".into();
        i.matcher.device = None;
        i.matcher.binary = Some("soundux".into());
        c.junction[0].input.push(i);
        c.junction[0].enabled = false;
    });
    keeps_all_comments(&text);
    assert!(text.contains("id = \"soundux\""), "{text}");
    assert!(text.contains("enabled = false"), "{text}");
    let text = write(&mut s, |c| {
        c.junction.retain(|j| j.id != "obs");
    });
    assert!(!text.contains("OBS Mix"), "{text}");
    assert!(text.contains("# The one I use for meetings."), "{text}");
}

#[test]
fn suppressing_a_link_writes_a_list() {
    let mut s = store();
    let text = write(&mut s, |c| c.junction[0].input[1].suppressed.push([1, 2]));
    assert!(text.contains("suppressed = [[1, 2]]"), "{text}");
    keeps_all_comments(&text);
}

#[test]
fn hand_edits_are_taken_or_rejected_with_a_position() {
    let mut s = store();
    assert_eq!(s.reload(FILE), Reload::Unchanged);
    let edited = FILE.replace("volume = 70", "volume = 40");
    assert_eq!(s.reload(&edited), Reload::Changed(2));
    assert_eq!(s.config().junction[0].input[0].volume, 40);
    // A typo: rejected, last good config stays, error points at the line.
    let broken = edited.replace("volume = 40", "volum = 40");
    let Reload::Invalid(d) = s.reload(&broken) else {
        panic!("should be invalid")
    };
    let loc = d[0].location.as_ref().expect("location");
    assert_eq!((d[0].code, loc.line), ("E_CONFIG_UNKNOWN_KEY", 12));
    assert_eq!(s.config().junction[0].input[0].volume, 40, "last good kept");
    assert_eq!(s.revision(), 2);
    assert!(s.error().is_some());
    assert_eq!(s.good_text(), edited);
    // Fixed again.
    assert_eq!(
        s.reload(&edited.replace("volume = 40", "volume = 45")),
        Reload::Changed(3)
    );
    assert!(s.error().is_none());
}

#[test]
fn comment_only_edits_do_not_bump_the_revision() {
    let mut s = store();
    assert_eq!(
        s.reload(&FILE.replace("# Streaming.", "# Streaming setup.")),
        Reload::Unchanged
    );
    assert_eq!(s.revision(), 1);
    // And the next write keeps the new comment.
    let text = write(&mut s, |c| c.junction[1].name = "OBS".into());
    assert!(text.contains("# Streaming setup."), "{text}");
}

#[test]
fn own_writes_are_not_reloaded() {
    let mut s = store();
    let text = write(&mut s, |c| c.junction[1].name = "Stream".into());
    assert_eq!(s.reload(&text), Reload::Unchanged);
}

#[test]
fn stale_revisions_conflict() {
    let mut s = store();
    let mut c = s.config().clone();
    c.junction[1].name = "A".into();
    s.reload(&FILE.replace("volume = 70", "volume = 10")); // someone edits meanwhile
    let err = s.update(Some(1), c).expect_err("conflict");
    assert_eq!(err[0].code, CONFLICT);
    assert_eq!(s.config().junction[1].name, "OBS Mix");
}

#[test]
fn replacing_the_file_keeps_the_given_text_and_checks_the_revision() {
    let mut s = store();
    let edited = FILE.replace("name = \"OBS Mix\"", "name = \"OBS\" # shorter");
    assert_eq!(s.replace(Some(1), &edited), Ok(2));
    assert_eq!(s.good_text(), edited);
    assert_eq!(s.config().junction[1].name, "OBS");
    assert_eq!(s.reload(&edited), Reload::Unchanged);

    // Comment-only replacement: same config, same revision.
    assert_eq!(s.replace(None, &format!("# hi\n{edited}")), Ok(2));

    assert_eq!(
        s.replace(Some(1), FILE).expect_err("stale")[0].code,
        CONFLICT
    );
    let bad = s
        .replace(None, "schema = 1\nfoo = 2\n")
        .expect_err("invalid");
    assert_eq!(bad[0].code, "E_CONFIG_UNKNOWN_KEY");
    assert_eq!(s.config().junction[1].name, "OBS");
}

#[test]
fn new_entries_are_written_without_defaults() {
    let mut s = store();
    let text = write(&mut s, |c| {
        let mut j = c.junction[1].clone();
        j.id = "cable".into();
        j.name = "Cable".into();
        j.kind = junc_core::model::Kind::Cable;
        let mut i = c.junction[0].input[0].clone();
        i.volume = 100;
        j.input.push(i);
        j.cable = Some(junc_core::model::Cable {
            to: junc_core::model::Match {
                binary: Some("obs".into()),
                ..Default::default()
            },
        });
        c.junction.push(j);
    });
    let tail = &text[text.find("id = \"cable\"").expect("new entry")..];
    assert_eq!(
        tail,
        "id = \"cable\"\nname = \"Cable\"\nkind = \"cable\"\ncable = { to = { binary = \"obs\" } }\n\n\
         [[junction.input]]\nid = \"spotify\"\nmatch = { binary = \"spotify\" }\n"
    );
}
