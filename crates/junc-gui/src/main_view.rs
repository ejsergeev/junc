// SPDX-License-Identifier: GPL-3.0-or-later

//! The Main screen's model (architecture §7.2, §7.3; design boards 1–6):
//! the selected Junction as cards in three columns and the links between
//! their ports, computed from juncd's `Describe`. Plain data without UI
//! text: the window formats counts and channel labels through its own
//! translations, so this is tested without a window and in any language.

use junc_core::model::{DefaultRole, Match};
use junc_core::plan::{default_map, gain_node_name, listen_node_name};
use junc_ipc::view::{Describe, InputView, JunctionView, ListenView, State};

/// What a card shows next to its title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// A music or media player.
    Music,
    /// Any other app.
    App,
    /// A hardware input.
    Mic,
    /// What a sink plays.
    Monitor,
    /// Another Junction.
    Junction,
    /// An output device.
    Speaker,
    /// A screen's output (HDMI, DisplayPort).
    Screen,
}

impl Icon {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Music => "music",
            Self::App => "app",
            Self::Mic => "mic",
            Self::Monitor => "monitor",
            Self::Junction => "junction",
            Self::Speaker => "speaker",
            Self::Screen => "screen",
        }
    }
}

/// A card's state beyond on and off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardState {
    Ok,
    /// A device Input or a Listen output whose device is away.
    Offline,
    /// An app Input whose app is not running.
    NotRunning,
}

impl CardState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Offline => "offline",
            Self::NotRunning => "not-running",
        }
    }
}

/// One channel row: its number (1-based) and position letter, and what its
/// meter listens to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    pub number: u8,
    /// `L`, `R` or empty.
    pub position: &'static str,
    /// `node.name` whose output port `number - 1` the meter reads.
    pub meter_node: String,
}

/// An Input or Listen card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub id: String,
    pub title: String,
    pub icon: Icon,
    pub enabled: bool,
    pub state: CardState,
    pub exclusive: bool,
    pub volume: u16,
    pub channels: Vec<Channel>,
}

/// A group of Bus channels: a stereo pair or a single channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusGroup {
    pub first: u8,
    pub channels: Vec<Channel>,
}

/// One end of a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// Input card `card`, channel `ch` (1-based), its output port.
    Input {
        card: usize,
        ch: u8,
    },
    /// Bus channel `ch`, its input port (left) or output port (right).
    BusIn {
        ch: u8,
    },
    BusOut {
        ch: u8,
    },
    /// Listen card `card`, channel `ch`, its input port.
    Listen {
        card: usize,
        ch: u8,
    },
}

/// A link between two ports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub from: End,
    pub to: End,
    /// Drawn dashed: its Input is waiting for its device or app.
    pub offline: bool,
    /// The Input or Listen id and the pair as the config writes it, to
    /// unlink it.
    pub owner: String,
    pub pair: [u8; 2],
}

/// The Main screen of one Junction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Main {
    pub id: String,
    pub name: String,
    pub node: String,
    pub enabled: bool,
    pub default: DefaultRole,
    pub inputs: Vec<Card>,
    pub bus: Vec<BusGroup>,
    pub listens: Vec<Card>,
    pub links: Vec<Link>,
    /// Inputs that are apps, devices (anything else), off, offline.
    pub apps: usize,
    pub devices: usize,
    pub off: usize,
    pub offline: usize,
}

/// Position letters for a node of `n` channels (stereo pairs read L, R).
fn position(n: u8, ch: u8) -> &'static str {
    match (n, ch) {
        (2, 1) => "L",
        (2, 2) => "R",
        _ => "",
    }
}

/// Apps whose card shows a note instead of the generic app glyph.
const MEDIA: &[&str] = &[
    "spotify",
    "rhythmbox",
    "lollypop",
    "elisa",
    "amberol",
    "clementine",
    "strawberry",
    "audacious",
    "mpv",
    "vlc",
    "cmus",
    "ncspot",
    "tidal-hifi",
];

fn input_icon(i: &InputView) -> Icon {
    match_icon(&i.matcher)
}

/// The icon of what `m` takes audio from.
pub fn match_icon(m: &Match) -> Icon {
    if m.device.is_some() {
        Icon::Mic
    } else if m.monitor.is_some() {
        Icon::Monitor
    } else if m.junction.is_some() {
        Icon::Junction
    } else {
        let binary = m.binary.as_deref().unwrap_or("").to_lowercase();
        let app = m.app.as_deref().unwrap_or("").to_lowercase();
        if MEDIA.iter().any(|x| *x == binary || *x == app) {
            Icon::Music
        } else {
            Icon::App
        }
    }
}

/// The name people see: the config's, else what the matched node calls
/// itself, else the match.
fn input_title(d: &Describe, i: &InputView) -> String {
    if let Some(n) = i.name.as_ref().filter(|n| !n.is_empty()) {
        return n.clone();
    }
    let m = &i.matcher;
    if let Some(j) = &m.junction
        && let Some(v) = d.junctions.iter().find(|v| &v.id == j)
    {
        return v.name.clone();
    }
    // The node it matched, else one it would match (an Input that is off
    // matches nothing, but its app may well be there).
    let would = |c: &&junc_ipc::view::Candidate| {
        m.binary.is_some() && c.binary == m.binary
            || m.app.is_some() && c.app == m.app
            || m.device.as_ref() == Some(&c.node)
            || m.node.as_ref() == Some(&c.node)
    };
    if let Some(c) = i
        .matched
        .first()
        .and_then(|n| d.candidates.iter().find(|c| &c.node == n))
        .or_else(|| d.candidates.iter().find(would))
    {
        if let Some(app) = c.app.as_ref().filter(|a| !a.is_empty()) {
            return app.clone();
        }
        if !c.description.is_empty() {
            return c.description.clone();
        }
    }
    if let Some(sink) = &m.monitor
        && let Some(c) = d.candidates.iter().find(|c| &c.node == sink)
    {
        return c.description.clone();
    }
    m.app
        .clone()
        .or_else(|| m.binary.clone())
        .or_else(|| m.device.clone())
        .or_else(|| m.monitor.clone())
        .or_else(|| m.node.clone())
        .or_else(|| m.role.clone())
        .unwrap_or_else(|| i.id.clone())
}

fn listen_title(d: &Describe, l: &ListenView) -> String {
    let node = if l.target == "default" {
        d.defaults.output.as_deref()
    } else {
        Some(l.target.as_str())
    };
    node.and_then(|n| d.candidates.iter().find(|c| c.node == n))
        .map(|c| c.description.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| l.target.clone())
}

/// Channels of a card from the links in use; two while unknown.
fn width(links: &[[u8; 2]], suppressed: &[[u8; 2]], side: usize) -> u8 {
    links
        .iter()
        .chain(suppressed)
        .map(|p| p[side])
        .max()
        .unwrap_or(2)
}

fn channels(n: u8, node: &str) -> Vec<Channel> {
    (1..=n)
        .map(|ch| Channel {
            number: ch,
            position: position(n, ch),
            meter_node: node.to_owned(),
        })
        .collect()
}

fn bus(j: &JunctionView) -> Vec<BusGroup> {
    let mut out = Vec::new();
    let mut first = 1u8;
    while first <= j.channels {
        let n = if first < j.channels { 2 } else { 1 };
        out.push(BusGroup {
            first,
            channels: (first..first + n)
                .map(|ch| Channel {
                    number: ch,
                    position: if n == 2 {
                        position(2, ch - first + 1)
                    } else {
                        ""
                    },
                    meter_node: j.node.clone(),
                })
                .collect(),
        });
        first = first.saturating_add(n);
        if first == 0 {
            break;
        }
    }
    out
}

fn input_card(d: &Describe, j: &JunctionView, i: &InputView) -> Card {
    let is_app =
        i.matcher.device.is_none() && i.matcher.monitor.is_none() && i.matcher.junction.is_none();
    let state = match i.status {
        State::Waiting if is_app => CardState::NotRunning,
        State::Waiting => CardState::Offline,
        _ => CardState::Ok,
    };
    let links = if i.links.is_empty() {
        default_map(2, usize::from(j.channels))
    } else {
        i.links.clone()
    };
    Card {
        id: i.id.clone(),
        title: input_title(d, i),
        icon: input_icon(i),
        enabled: i.enabled,
        state,
        exclusive: i.exclusive,
        volume: i.volume,
        channels: channels(
            width(&links, &i.suppressed, 0),
            &gain_node_name(&j.id, &i.id),
        ),
    }
}

fn listen_card(d: &Describe, j: &JunctionView, l: &ListenView) -> Card {
    Card {
        id: l.id.clone(),
        title: listen_title(d, l),
        icon: Icon::Speaker,
        enabled: l.enabled,
        state: if l.status == State::Waiting {
            CardState::Offline
        } else {
            CardState::Ok
        },
        exclusive: false,
        volume: l.volume,
        channels: channels(
            width(&l.links, &l.suppressed, 1),
            &listen_node_name(&j.id, &l.id),
        ),
    }
}

/// The Main screen of Junction `id`, or `None` if it is not in `d`.
pub fn main(d: &Describe, id: &str) -> Option<Main> {
    let j = d.junctions.iter().find(|j| j.id == id)?;
    let inputs: Vec<Card> = j.inputs.iter().map(|i| input_card(d, j, i)).collect();
    let listens: Vec<Card> = j.listens.iter().map(|l| listen_card(d, j, l)).collect();
    let mut links = Vec::new();
    // Off, the Junction carries nothing: no links (board 18).
    for (card, i) in j.inputs.iter().enumerate().filter(|_| j.enabled) {
        if !i.enabled {
            continue;
        }
        let waiting = i.status == State::Waiting;
        let pairs = if i.links.is_empty() && waiting {
            default_map(2, usize::from(j.channels))
        } else {
            i.links.clone()
        };
        for p in pairs.iter().filter(|p| !i.suppressed.contains(p)) {
            links.push(Link {
                from: End::Input { card, ch: p[0] },
                to: End::BusIn { ch: p[1] },
                offline: waiting,
                owner: i.id.clone(),
                pair: *p,
            });
        }
    }
    for (card, l) in j.listens.iter().enumerate().filter(|_| j.enabled) {
        if !l.enabled {
            continue;
        }
        for p in l.links.iter().filter(|p| !l.suppressed.contains(p)) {
            links.push(Link {
                from: End::BusOut { ch: p[0] },
                to: End::Listen { card, ch: p[1] },
                offline: l.status == State::Waiting,
                owner: l.id.clone(),
                pair: *p,
            });
        }
    }
    let apps = inputs
        .iter()
        .filter(|c| matches!(c.icon, Icon::Music | Icon::App))
        .count();
    Some(Main {
        id: j.id.clone(),
        name: j.name.clone(),
        node: j.node.clone(),
        enabled: j.enabled,
        default: j.default,
        apps,
        devices: inputs.len() - apps,
        off: inputs.iter().filter(|c| !c.enabled).count(),
        offline: inputs
            .iter()
            .filter(|c| c.enabled && c.state == CardState::Offline)
            .count(),
        inputs,
        bus: bus(j),
        listens,
        links,
    })
}

/// What a Cable's screen (board 12) shows beyond its Main model: the app
/// it delivers to, and its own master volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CableTo {
    /// The app's name, else what the config matches.
    pub name: String,
    /// The app is running and hears the Cable.
    pub running: bool,
    pub volume: u16,
}

/// The app Junction `id` delivers to, when it is a Cable.
pub fn cable_to(d: &Describe, id: &str) -> Option<CableTo> {
    let j = d.junctions.iter().find(|j| j.id == id)?;
    let to = j.cable_to.as_ref()?;
    let name = d
        .candidates
        .iter()
        .filter(|c| c.kind == "app_capture")
        .find(|c| to.binary.is_some() && c.binary == to.binary)
        .and_then(|c| c.app.clone())
        .or_else(|| to.app.clone())
        .or_else(|| to.binary.clone())
        .unwrap_or_default();
    Some(CableTo {
        name,
        running: j.cable_target == Some(State::Active),
        volume: j.volume,
    })
}

/// Per channel of Input card `card`: the Bus channel it feeds (the first,
/// when several), 0 for none (Input Options, board 15).
pub fn feeds(m: &Main, card: usize) -> Vec<i32> {
    let Some(c) = m.inputs.get(card) else {
        return Vec::new();
    };
    c.channels
        .iter()
        .map(|ch| {
            m.links
                .iter()
                .find_map(|l| match (l.from, l.to) {
                    (End::Input { card: k, ch: n }, End::BusIn { ch: b })
                        if k == card && n == ch.number =>
                    {
                        Some(i32::from(b))
                    }
                    _ => None,
                })
                .unwrap_or(0)
        })
        .collect()
}

impl Main {
    /// Nodes the column meters read, without repeats.
    pub fn metered(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let all = self
            .inputs
            .iter()
            .chain(&self.listens)
            .flat_map(|c| &c.channels)
            .chain(self.bus.iter().flat_map(|g| &g.channels));
        for ch in all {
            if !out.contains(&ch.meter_node) {
                out.push(ch.meter_node.clone());
            }
        }
        out
    }
}

/// What deleting a Junction removes with it (board 22).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    pub inputs: usize,
    pub links: usize,
    /// Bus groups: first channel, stereo.
    pub bus: Vec<(u8, bool)>,
    pub listens: usize,
    /// Its Inputs' names: they keep playing as before.
    pub names: Vec<String>,
}

/// What deleting `m` removes.
pub fn removal(m: &Main) -> Removal {
    Removal {
        inputs: m.inputs.len(),
        links: m.links.len(),
        bus: m
            .bus
            .iter()
            .map(|g| (g.first, g.channels.len() == 2))
            .collect(),
        listens: m.listens.len(),
        names: m.inputs.iter().map(|c| c.title.clone()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn golden() -> Describe {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/golden/describe/full.json"
        ))
        .expect("golden");
        let v: serde_json::Value = serde_json::from_str(&text).expect("json");
        serde_json::from_value(v["data"].clone()).expect("describe")
    }

    #[test]
    fn cards_follow_the_config_and_the_links_in_use() {
        let m = main(&golden(), "virtual-mic-for-zoom").expect("junction");
        let titles: Vec<&str> = m.inputs.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Spotify",
                "soundux",
                "alsa_input.usb-Generic_USB_Microphone-00.mono-fallback"
            ],
            "app name, binary, then the device as a last resort"
        );
        let icons: Vec<Icon> = m.inputs.iter().map(|c| c.icon).collect();
        assert_eq!(icons, [Icon::Music, Icon::App, Icon::Mic]);
        assert_eq!((m.apps, m.devices, m.off, m.offline), (2, 1, 0, 0));
        assert_eq!(
            m.inputs[2].channels.len(),
            1,
            "a mono mic linked to both sides"
        );
        assert_eq!(m.inputs[0].channels[1].position, "R");
        assert_eq!(
            m.inputs[0].channels[0].meter_node,
            "junc.virtual_mic_for_zoom.in.spotify"
        );
        let groups: Vec<(u8, usize)> = m.bus.iter().map(|g| (g.first, g.channels.len())).collect();
        assert_eq!(groups, [(1, 2), (3, 2)]);
        assert_eq!(m.bus[1].channels[0].position, "L");
        let to_bus: Vec<(End, End)> = m
            .links
            .iter()
            .filter(|l| l.owner == "usb-mic")
            .map(|l| (l.from, l.to))
            .collect();
        assert_eq!(
            to_bus,
            [(End::Input { card: 2, ch: 1 }, End::BusIn { ch: 1 })],
            "1 → 2 was removed by another tool and left so (suppressed)"
        );
        assert!(
            m.links
                .iter()
                .any(|l| l.from == End::BusOut { ch: 2 } && l.to == End::Listen { card: 0, ch: 2 })
        );
        assert_eq!(
            m.metered(),
            [
                "junc.virtual_mic_for_zoom.in.spotify",
                "junc.virtual_mic_for_zoom.in.soundux",
                "junc.virtual_mic_for_zoom.in.usb_mic",
                "junc.virtual_mic_for_zoom.listen.builtin",
                "junc.virtual_mic_for_zoom"
            ]
        );
        assert!(main(&golden(), "nope").is_none());
    }

    #[test]
    fn deleting_removes_its_inputs_links_bus_and_listens() {
        let m = main(&golden(), "virtual-mic-for-zoom").expect("junction");
        let r = removal(&m);
        assert_eq!((r.inputs, r.links, r.listens), (3, m.links.len(), 1));
        assert_eq!(r.bus, [(1, true), (3, true)]);
        assert_eq!(r.names[0], "Spotify");
    }

    #[test]
    fn off_and_waiting_inputs_show_as_such() {
        let mut d = golden();
        let j = &mut d.junctions[0];
        j.inputs[1].enabled = false;
        j.inputs[2].status = State::Waiting;
        j.inputs[2].links.clear();
        j.inputs[2].name = Some("USB Microphone".into());
        j.inputs[0].status = State::Waiting;
        let m = main(&d, "virtual-mic-for-zoom").expect("junction");
        assert_eq!(m.inputs[2].title, "USB Microphone");
        assert_eq!(m.inputs[2].state, CardState::Offline);
        assert_eq!(m.inputs[0].state, CardState::NotRunning);
        assert_eq!((m.off, m.offline), (1, 1));
        assert!(
            m.links.iter().all(|l| l.owner != "soundux"),
            "an Input that is off draws no links"
        );
        let mic: Vec<&Link> = m.links.iter().filter(|l| l.owner == "usb-mic").collect();
        assert_eq!(mic.len(), 2, "waiting: the default stereo mapping, dashed");
        assert!(mic.iter().all(|l| l.offline));
    }

    #[test]
    fn an_off_junction_keeps_its_cards_but_draws_no_links() {
        let mut d = golden();
        d.junctions[0].enabled = false;
        let m = main(&d, "virtual-mic-for-zoom").expect("junction");
        assert!(!m.enabled);
        assert_eq!((m.inputs.len(), m.listens.len()), (3, 1));
        assert!(
            m.inputs.iter().all(|c| c.enabled),
            "its Inputs keep their switches"
        );
        assert!(m.links.is_empty(), "apps see silence: nothing is linked");
    }
}
