// SPDX-License-Identifier: GPL-3.0-or-later

//! The window's controller (architecture §7.1): keeps juncd's latest
//! `Describe`, the selected Junction, its Main model and layout, the link
//! under the pointer, onboarding and the Add Input picker, and turns the
//! window's callbacks into [`Command`]s. Everything runs on the GUI thread.

use crate::actions::{Actions, Command};
use crate::checks;
use crate::edits::{Edit, Part};
use crate::layout::{self, Layout, Point};
use crate::linking;
use crate::main_view::{self, End, Icon, Main};
use crate::meter_service::{Levels, MeterService};
use crate::onboarding::{self, Step};
use crate::picker::{self, Entry, Pick};
use crate::state::{self, Snapshot};
use crate::ui::{self, AppWindow, View, Words};
use junc_core::model::{DefaultRole, Kind, Match};
use junc_ipc::view::{Describe, InputView, JunctionView, State};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

/// Volume sliders send their value once it rests this long.
const SETTLE: Duration = Duration::from_millis(250);

/// The controller.
pub struct App {
    window: slint::Weak<AppWindow>,
    actions: Rc<dyn Actions>,
    meters: Option<Rc<MeterService>>,
    levels: Levels,
    describe: Option<Describe>,
    sidebar: Snapshot,
    selected: Option<String>,
    main: Option<Main>,
    layout: Option<Layout>,
    /// The canvas width, set by the window (also while the controller is
    /// busy, so it lives outside the RefCell).
    width: Rc<Cell<f32>>,
    hovered: Option<usize>,
    /// Volumes waiting for their slider to rest, by what they change.
    pending: Rc<RefCell<BTreeMap<String, Command>>>,
    timer: slint::Timer,
    /// Onboarding's step on screen, if any.
    onboarding: Option<Step>,
    /// Where onboarding is remembered as done; `None` never onboards
    /// (scenarios set the step themselves).
    state_file: Option<PathBuf>,
    /// The system check's last answer.
    checks: Vec<checks::Row>,
    /// The Junction ids before a New Junction, to select the new one.
    creating: Option<Vec<String>>,
    /// The picker's rows and those the search shows, when it is open.
    picker: Option<(Vec<Entry>, Vec<usize>)>,
    /// What the open picker picks.
    picking: Picking,
    /// The Input card whose Options panel is open (boards 15, 30).
    options: Option<usize>,
    /// A link being made (boards 13, 14).
    linking: Option<Linking>,
    /// juncd's last snapshot as it came, before the draft is added.
    received: Option<Describe>,
    /// A new Cable held until both its apps are chosen (board 27).
    draft: Option<Draft>,
}

/// The sidebar id of the draft Cable (a config id has no `+`).
pub const DRAFT: &str = "+new-cable";

/// A new Cable the window holds: the config needs both apps (board 27).
#[derive(Debug, Clone)]
struct Draft {
    name: String,
    /// The app it takes from, and its name.
    from: Option<(Match, String)>,
    to: Option<(Match, String)>,
}

/// The draft as juncd would show it, not routed yet.
fn draft_view(d: &Draft) -> JunctionView {
    JunctionView {
        id: DRAFT.to_owned(),
        name: d.name.clone(),
        kind: Kind::Cable,
        enabled: true,
        volume: 100,
        default: DefaultRole::None,
        channels: 2,
        node: format!(
            "junc.{}",
            junc_core::config::slugify(&d.name).replace('-', "_")
        ),
        path: String::new(),
        status: State::Unknown,
        inputs: d
            .from
            .iter()
            .map(|(m, label)| InputView {
                id: junc_core::config::slugify(label),
                matcher: m.clone(),
                name: Some(label.clone()),
                enabled: true,
                volume: 100,
                exclusive: false,
                status: State::Active,
                matched: Vec::new(),
                links: Vec::new(),
                custom_links: false,
                suppressed: Vec::new(),
            })
            .collect(),
        listens: Vec::new(),
        cable_target: d.to.as_ref().map(|_| State::Active),
        cable_to: d.to.as_ref().map(|(m, _)| m.clone()),
    }
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("selected", &self.selected)
            .field("hovered", &self.hovered)
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

/// A link being made: from a port, to the target it would end at; by the
/// pointer (at `pointer`) or the keys.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Linking {
    from: End,
    target: Option<End>,
    keys: bool,
    pointer: Point,
}

/// What the picker picks: Inputs (board 7), Listen outputs (board 17), a
/// Cable's apps (board 12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Picking {
    Input,
    Listen,
    CableFrom,
    CableTo,
}

/// Puts `rows` into `current` in place when the counts match, so the
/// window keeps the elements (and the focus) it made for them; else sets a
/// new model.
fn update<T: Clone + PartialEq + 'static>(
    current: ModelRc<T>,
    rows: Vec<T>,
    set: impl FnOnce(ModelRc<T>),
) {
    if current.row_count() != rows.len() {
        set(ModelRc::new(VecModel::from(rows)));
        return;
    }
    for (i, row) in rows.into_iter().enumerate() {
        if current.row_data(i).as_ref() != Some(&row) {
            current.set_row_data(i, row);
        }
    }
}

/// Wires `window`'s callbacks to a new controller.
pub fn install(
    window: &AppWindow,
    actions: Rc<dyn Actions>,
    meters: Option<Rc<MeterService>>,
    levels: Levels,
) -> Rc<RefCell<App>> {
    let view = window.global::<View>();
    view.set_levels(ModelRc::from(levels.levels.clone()));
    view.set_peaks(ModelRc::from(levels.peaks.clone()));
    view.set_version(env!("CARGO_PKG_VERSION").into());
    let width = Rc::new(Cell::new(0.0));
    let app = Rc::new(RefCell::new(App {
        window: window.as_weak(),
        actions,
        meters,
        levels,
        describe: None,
        sidebar: state::snapshot(None),
        selected: None,
        main: None,
        layout: None,
        width: width.clone(),
        hovered: None,
        pending: Rc::default(),
        timer: slint::Timer::default(),
        onboarding: None,
        state_file: None,
        checks: Vec::new(),
        creating: None,
        received: None,
        draft: None,
        picker: None,
        picking: Picking::Input,
        options: None,
        linking: None,
    }));
    // Runs `f` on the controller unless it is busy (a callback raised while
    // it updates the window, such as a relayout when the canvas appears).
    let w = Rc::downgrade(&app);
    let call = move |f: &dyn Fn(&mut App)| {
        if let Some(a) = w.upgrade()
            && let Ok(mut a) = a.try_borrow_mut()
        {
            f(&mut a);
        }
    };
    let call = Rc::new(call);

    {
        let c = call.clone();
        view.on_select(move |id| c(&|a| a.select(id.as_str())));
    }
    {
        let c = call.clone();
        view.on_junction_toggled(move |id, on| {
            c(&|a| {
                if let Some(path) = a.path(id.as_str()) {
                    a.actions.send(Command::Enabled { path, on });
                }
            });
        });
    }
    {
        let c = call.clone();
        view.on_junction_volume(move |id, v| {
            c(&|a| {
                if let Some(path) = a.path(id.as_str()) {
                    let percent = u16::try_from(v).unwrap_or(0);
                    a.later(format!("j/{id}"), Command::Volume { path, percent });
                }
            });
        });
    }
    {
        let (c, width) = (call.clone(), width);
        view.on_relayout(move |w| {
            width.set(w);
            c(&App::relayout);
        });
    }
    {
        let c = call.clone();
        view.on_card_toggled(move |column, i, on| {
            c(&|a| {
                if let Some((junction, part)) = a.part(column, i) {
                    a.actions.send(Command::Edit {
                        junction,
                        edit: Edit::Enabled(part, on),
                    });
                }
            });
        });
    }
    {
        let c = call.clone();
        view.on_card_volume(move |column, i, v| {
            c(&|a| {
                if let Some((junction, part)) = a.part(column, i) {
                    let key = format!("{junction}/{part:?}");
                    let edit = Edit::Volume(part, u16::try_from(v).unwrap_or(0));
                    a.later(key, Command::Edit { junction, edit });
                }
            });
        });
    }
    {
        let c = call.clone();
        view.on_pointer_moved(move |x, y| c(&|a| a.hover(Some(Point { x, y }))));
    }
    {
        let c = call.clone();
        view.on_pointer_left(move || c(&|a| a.hover(None)));
    }
    {
        let c = call.clone();
        view.on_link_clicked(move || {
            c(&|a| {
                if let Some(i) = a.hovered {
                    a.unlink(&[i]);
                }
            });
        });
    }
    {
        let c = call.clone();
        view.on_port_remove(move |column, card, ch, side| {
            c(&|a| a.remove_port(column, card, ch, side));
        });
    }
    {
        let c = call.clone();
        view.on_recheck(move || c(&App::recheck));
        let c = call.clone();
        view.on_open_settings(move || c(&App::open_settings));
        let c = call.clone();
        view.on_start_at_login_changed(move |on| {
            c(&|a| a.actions.send(Command::Service(Some(on))));
        });
        let c = call.clone();
        view.on_follow_theme_changed(move |on| c(&|a| a.follow_theme(on)));
        let c = call.clone();
        view.on_open_source(move || {
            c(&|a| {
                a.actions
                    .send(Command::Open(env!("CARGO_PKG_REPOSITORY").to_owned()));
            });
        });
        let c = call.clone();
        view.on_open_logs(move || c(&|a| a.actions.send(Command::OpenLogs)));
    }
    {
        let c = call.clone();
        view.on_copy(move |t| c(&|a| a.actions.send(Command::Copy(t.to_string()))));
    }
    {
        let c = call.clone();
        view.on_onboarding_continue(move || {
            c(&|a| {
                if checks::passed(&a.checks) {
                    a.set_step(Some(Step::Templates));
                }
            });
        });
    }
    {
        let c = call.clone();
        view.on_onboarding_back(move || c(&|a| a.set_step(Some(Step::Check))));
    }
    {
        let c = call.clone();
        view.on_template_chosen(move |id| c(&|a| a.template(id.as_str())));
    }
    {
        let c = call.clone();
        view.on_new_junction(move || c(&App::new_junction));
        let c = call.clone();
        view.on_new_cable(move || c(&App::new_cable));
    }
    {
        let c = call.clone();
        view.on_open_picker(move || c(&App::open_picker));
        let c = call.clone();
        view.on_open_listen_picker(move || c(&App::open_listen_picker));
        let c = call.clone();
        view.on_card_arrow(move |i, dx, dy| {
            c(&|a| {
                let Some(w) = a.window.upgrade() else { return };
                let view = w.global::<View>();
                let cards: Vec<_> = view.get_cards().iter().collect();
                if let Some(j) = usize::try_from(i)
                    .ok()
                    .and_then(|i| ui::arrow_target(&cards, i, dx, dy))
                {
                    view.set_focus_card(i32::try_from(j).unwrap_or(-1));
                }
            });
        });
        let c = call.clone();
        view.on_link_start(move |col, card, ch, side| {
            c(&|a| {
                if let Some(e) = ui::port_end(col, card, ch, side) {
                    a.link_start(e, false);
                }
            });
        });
        let c = call.clone();
        view.on_link_key(move |col, card, ch, side| {
            c(&|a| {
                if let Some(e) = ui::port_end(col, card, ch, side) {
                    a.link_key(e);
                }
            });
        });
        let c = call.clone();
        view.on_link_drag(move |x, y| c(&|a| a.link_drag(Point { x, y })));
        let c = call.clone();
        view.on_link_drop(move || c(&App::link_finish));
        let c = call.clone();
        view.on_link_step(move |d| c(&|a| a.link_step(d)));
        let c = call.clone();
        view.on_link_cancel(move || c(&App::link_cancel));
        let c = call.clone();
        view.on_open_cable_picker(move |to| c(&|a| a.open_cable_picker(to)));
        let c = call.clone();
        view.on_cable_exclusive_changed(move |on| c(&|a| a.cable_exclusive(on)));
        let c = call.clone();
        view.on_cable_listen_toggled(move |on, i| c(&|a| a.cable_listen(on, i)));
        let c = call.clone();
        view.on_cable_listen_target(move |i| c(&|a| a.cable_listen_target(i)));
        let c = call.clone();
        view.on_options_toggled(move |i| c(&|a| a.toggle_options(i)));
        let c = call.clone();
        view.on_options_closed(move || c(&App::close_options));
        let c = call.clone();
        view.on_options_exclusive_changed(move |on| c(&|a| a.options_exclusive(on)));
        let c = call.clone();
        view.on_options_feed(move |ch, bus| c(&|a| a.options_feed(ch, bus)));
        let c = call.clone();
        view.on_options_remove(move || c(&App::options_remove));
        let c = call.clone();
        view.on_add_bus(move |n| {
            c(&|a| {
                if let (Some(m), Ok(n)) = (&a.main, u8::try_from(n)) {
                    a.actions.send(Command::Edit {
                        junction: m.id.clone(),
                        edit: Edit::AddBus(n),
                    });
                }
            });
        });
    }
    {
        let c = call.clone();
        view.on_picker_search_edited(move |t| c(&|a| a.search(t.as_str())));
    }
    {
        let c = call.clone();
        view.on_picker_picked(move |i| c(&|a| a.pick(i)));
    }
    {
        let c = call.clone();
        view.on_picker_closed(move || c(&App::close_picker));
    }
    {
        let c = call.clone();
        view.on_rename(move |name| c(&|a| a.rename(name.as_str())));
    }
    {
        let c = call.clone();
        view.on_duplicate(move |id| c(&|a| a.duplicate(id.as_str())));
    }
    {
        let c = call;
        view.on_delete_junction(move || {
            c(&|a| {
                if a.selected.as_deref() == Some(DRAFT) {
                    a.draft = None;
                    a.redraw();
                    return;
                }
                if let Some(path) = a.selected.clone().and_then(|id| a.path(&id)) {
                    a.actions.send(Command::Delete { path });
                }
            });
        });
    }
    app
}

impl App {
    /// juncd's new snapshot, or `None` while it is away.
    pub fn update(&mut self, describe: Option<Describe>) {
        // Board 20: PipeWire went away; doctor says what is wrong.
        let waiting = |d: &Option<Describe>| {
            d.as_ref()
                .is_some_and(|d| d.state == "waiting_for_pipewire")
        };
        if waiting(&describe) && !waiting(&self.received) {
            self.recheck();
        }
        self.received.clone_from(&describe);
        let describe = describe.map(|mut d| {
            d.junctions.extend(self.draft.iter().map(draft_view));
            d
        });
        self.sidebar = state::snapshot(describe.as_ref());
        self.describe = describe;
        let ids: Vec<&str> = self
            .sidebar
            .junctions
            .iter()
            .map(|r| r.id.as_str())
            .collect();
        // A New Junction selects itself once juncd shows it.
        if let Some(before) = &self.creating
            && let Some(new) = ids.iter().find(|id| !before.iter().any(|b| b == *id))
        {
            self.selected = Some((*new).to_owned());
            self.creating = None;
        }
        if self.selected.as_deref().is_none_or(|s| !ids.contains(&s)) {
            self.selected = ids.first().map(|s| (*s).to_owned());
        }
        if let Some(file) = &self.state_file {
            let done = onboarding::done(file);
            let needed = onboarding::needed(done, self.describe.as_ref());
            if self.onboarding.is_none() && needed {
                self.set_step(Some(Step::Check));
            } else if self.onboarding.is_some() && !needed {
                // Set up meanwhile (the CLI, an agent): nothing to onboard.
                self.finish_onboarding();
            }
        }
        self.refresh();
    }

    /// Onboards on a first launch, remembering it in `file`
    /// ([`onboarding::state_file`]); decided with juncd's next snapshot.
    pub fn onboard_with(&mut self, file: PathBuf) {
        self.state_file = Some(file);
    }

    /// Shows onboarding at `step` (scenarios), or no onboarding.
    pub fn set_step(&mut self, step: Option<Step>) {
        let first = self.onboarding.is_none() && step == Some(Step::Check);
        self.onboarding = step;
        if let Some(w) = self.window.upgrade() {
            w.global::<View>().set_onboarding(match step {
                None => 0,
                Some(Step::Check) => 1,
                Some(Step::Templates) => 2,
            });
        }
        if first {
            self.recheck();
        }
    }

    /// The onboarding step on screen.
    pub fn step(&self) -> Option<Step> {
        self.onboarding
    }

    fn finish_onboarding(&mut self) {
        if let Some(file) = &self.state_file
            && let Err(e) = onboarding::mark_done(file)
        {
            eprintln!("junc-gui: {}: {e}", file.display());
        }
        self.set_step(None);
    }

    fn recheck(&mut self) {
        if let Some(w) = self.window.upgrade() {
            w.global::<View>().set_checking(true);
        }
        self.actions.send(Command::Check);
    }

    /// The system check's answer.
    pub fn set_checks(&mut self, rows: Vec<checks::Row>) {
        if let Some(w) = self.window.upgrade() {
            let view = w.global::<View>();
            view.set_checking(false);
            view.set_checks_passed(checks::passed(&rows));
            view.set_checks(ModelRc::new(VecModel::from(ui::checks(&rows))));
            // Board 20 shows the audio server and its session manager.
            let service: Vec<checks::Row> = rows
                .iter()
                .filter(|r| r.id == "pipewire" || r.id == "wireplumber")
                .cloned()
                .collect();
            view.set_service_checks(ModelRc::new(VecModel::from(ui::checks(&service))));
        }
        self.checks = rows;
    }

    /// A starting point from onboarding. "Start empty" is a New Junction;
    /// templates with slots ask for them first (board 49, B22). Juncd
    /// starts at login from then on (F9, on by default after onboarding).
    fn template(&mut self, id: &str) {
        if id == "start-empty" {
            self.new_junction();
            self.finish_onboarding();
            self.actions.send(Command::Service(Some(true)));
        }
    }

    /// Board 25: Settings in the main area; Start at login is asked
    /// of systemd.
    pub fn open_settings(&mut self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        self.close_picker();
        let view = w.global::<View>();
        view.set_service_known(false);
        view.set_settings_open(true);
        self.actions.send(Command::Service(None));
    }

    /// What systemd says of Start at login (`None`: it could not say).
    pub fn set_service(&mut self, enabled: Option<bool>) {
        if let Some(w) = self.window.upgrade() {
            let view = w.global::<View>();
            view.set_service_known(enabled.is_some());
            view.set_start_at_login(enabled.unwrap_or(false));
        }
    }

    /// Board 25: follow the Omarchy theme or use the built-in one; kept
    /// in the GUI's state file.
    fn follow_theme(&mut self, on: bool) {
        if let Some(file) = &self.state_file
            && let Err(e) = onboarding::set_flag(file, "follow_theme", on)
        {
            eprintln!("junc-gui: {}: {e}", file.display());
        }
        if let Some(w) = self.window.upgrade() {
            w.global::<View>().set_follow_theme(on);
        }
        THEME.with(|t| {
            if let Some(f) = t.borrow().as_ref() {
                f(on);
            }
        });
    }

    /// Shows the draft's change at once (nothing goes to juncd).
    fn redraw(&mut self) {
        let d = self.received.clone();
        self.update(d);
    }

    /// Board 27: a new Cable, held by the window until both apps are
    /// chosen; the picker of the app it takes from opens.
    pub fn new_cable(&mut self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        if self.draft.is_none() {
            self.draft = Some(Draft {
                name: w.global::<Words>().invoke_new_cable().to_string(),
                from: None,
                to: None,
            });
        }
        self.select(DRAFT);
        self.redraw();
        self.open_cable_picker(false);
    }

    /// Board 27: the draft takes the picked app; with both it is written
    /// as a Cable and selected once juncd shows it.
    fn pick_for_draft(&mut self, pick: &Pick, label: &str) {
        let Some(d) = self.draft.as_mut() else { return };
        match pick {
            Pick::CableFrom(m) => d.from = Some((m.clone(), label.to_owned())),
            Pick::CableTo(m) => d.to = Some((m.clone(), label.to_owned())),
            Pick::Input(_) | Pick::Listen(_) => return,
        }
        self.close_picker();
        if let Some(Draft {
            name,
            from: Some((from, label)),
            to: Some((to, _)),
        }) = self.draft.clone()
        {
            self.creating = Some(
                self.sidebar
                    .junctions
                    .iter()
                    .filter(|r| r.id != DRAFT)
                    .map(|r| r.id.clone())
                    .collect(),
            );
            self.draft = None;
            self.actions.send(Command::Edit {
                junction: String::new(),
                edit: Edit::NewCable {
                    name,
                    from: Box::new(from),
                    label,
                    to: Box::new(to),
                },
            });
            self.redraw();
            return;
        }
        self.redraw();
        if self.draft.as_ref().is_some_and(|d| d.to.is_none()) {
            self.open_cable_picker(true);
        } else if self.draft.as_ref().is_some_and(|d| d.from.is_none()) {
            self.open_cable_picker(false);
        }
    }

    fn new_junction(&mut self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let name = w.global::<Words>().invoke_new_junction().to_string();
        self.creating = Some(
            self.sidebar
                .junctions
                .iter()
                .map(|r| r.id.clone())
                .collect(),
        );
        self.actions.send(Command::Create { name });
    }

    /// Opens the Add Input picker on the selected Junction.
    pub fn open_picker(&mut self) {
        self.open(Picking::Input);
    }

    /// Opens the picker of the app a Cable takes from (`to` false) or
    /// delivers to.
    pub fn open_cable_picker(&mut self, to: bool) {
        self.open(if to {
            Picking::CableTo
        } else {
            Picking::CableFrom
        });
    }

    /// Opens the Add Bus channels popover on the selected Junction.
    /// Board 26: gives the selected Junction a new name. A blank name or
    /// the same one changes nothing.
    pub fn rename(&mut self, name: &str) {
        let name = name.trim();
        let Some(id) = self.selected.clone() else {
            return;
        };
        if id == DRAFT {
            if let Some(d) = self.draft.as_mut().filter(|_| !name.is_empty()) {
                d.name = name.to_owned();
            }
            self.redraw();
            return;
        }
        let same = self
            .sidebar
            .junctions
            .iter()
            .any(|r| r.id == id && r.name == name);
        if name.is_empty() || same {
            return;
        }
        if let Some(path) = self.path(&id) {
            self.actions.send(Command::Rename {
                path,
                name: name.to_owned(),
            });
        }
    }

    /// Board 21: a copy of Junction `id` after it, named "<name> copy".
    pub fn duplicate(&self, id: &str) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let Some(row) = self.sidebar.junctions.iter().find(|r| r.id == id) else {
            return;
        };
        let name = w.global::<Words>().invoke_copy_of(row.name.as_str().into());
        self.actions.send(Command::Edit {
            junction: id.to_owned(),
            edit: Edit::Duplicate(name.into()),
        });
    }

    /// Board 21: the selected row's context menu with the cursor on item
    /// `cursor`, as the Menu key opens it.
    pub fn open_menu(&self, cursor: i32) {
        let (Some(id), Some(w)) = (&self.selected, self.window.upgrade()) else {
            return;
        };
        if let Some(i) = self.sidebar.junctions.iter().position(|r| &r.id == id) {
            let view = w.global::<View>();
            view.set_menu_cursor(cursor);
            view.set_menu_index(i32::try_from(i).unwrap_or(-1));
        }
    }

    /// Board 26: the name field in the header, as F2 opens it.
    pub fn start_rename(&self) {
        if let (Some(_), Some(w)) = (&self.selected, self.window.upgrade()) {
            w.global::<View>().set_renaming(true);
        }
    }

    /// Board 22: asks before deleting the selected Junction, as
    /// Ctrl+Delete does.
    pub fn start_delete(&self) {
        if let (Some(_), Some(w)) = (&self.selected, self.window.upgrade()) {
            w.global::<View>().set_confirming(true);
        }
    }

    pub fn open_add_bus(&self) {
        if let (Some(_), Some(w)) = (&self.main, self.window.upgrade()) {
            w.global::<View>().set_adding_bus(true);
        }
    }

    /// Opens the Add Listen output picker on the selected Junction.
    pub fn open_listen_picker(&mut self) {
        self.open(Picking::Listen);
    }

    fn open(&mut self, picking: Picking) {
        if self.main.is_none() {
            return;
        }
        self.picker = Some((Vec::new(), Vec::new()));
        self.picking = picking;
        if let Some(w) = self.window.upgrade() {
            let view = w.global::<View>();
            view.set_picker_search("".into());
            view.set_picker_cursor(0);
            view.set_picker_listen(picking == Picking::Listen);
            view.set_picker_cable(match picking {
                Picking::CableFrom => 1,
                Picking::CableTo => 2,
                Picking::Input | Picking::Listen => 0,
            });
        }
        self.load_picker();
        if let Some(w) = self.window.upgrade() {
            w.global::<View>().set_picker_open(true);
        }
    }

    /// The picker's rows from the latest snapshot, filtered by the search.
    fn load_picker(&mut self) {
        let (Some(d), Some(id), Some(w)) = (&self.describe, &self.selected, self.window.upgrade())
        else {
            return;
        };
        let view = w.global::<View>();
        let entries = match self.picking {
            Picking::Input => picker::entries(d, id),
            Picking::Listen => picker::outputs(d, id),
            Picking::CableFrom => picker::cable_apps(d, id, false),
            Picking::CableTo => picker::cable_apps(d, id, true),
        };
        let shown = picker::filter(&entries, view.get_picker_search().as_str());
        let items = ui::picker(&w.global::<Words>(), &entries, &shown);
        view.set_picker_items(ModelRc::new(VecModel::from(items)));
        self.picker = Some((entries, shown));
    }

    fn search(&mut self, _text: &str) {
        self.load_picker();
    }

    fn pick(&mut self, i: i32) {
        let (Some((entries, shown)), Some(m)) = (&self.picker, &self.main) else {
            return;
        };
        let Some(e) = usize::try_from(i)
            .ok()
            .and_then(|i| shown.get(i))
            .and_then(|&i| entries.get(i))
        else {
            return;
        };
        if e.added {
            return;
        }
        if m.id == DRAFT {
            let (pick, label) = (e.pick.clone(), e.label.clone());
            self.pick_for_draft(&pick, &label);
            return;
        }
        let edit = match &e.pick {
            Pick::Input(matcher) => Edit::AddInput {
                matcher: Box::new(matcher.clone()),
                label: e.label.clone(),
            },
            Pick::CableFrom(matcher) => Edit::CableFrom(Box::new(matcher.clone())),
            Pick::CableTo(matcher) => Edit::CableTo(Box::new(matcher.clone())),
            Pick::Listen(target) => Edit::AddListen {
                target: target.clone(),
                label: if e.label.is_empty() {
                    target.clone()
                } else {
                    e.label.clone()
                },
            },
        };
        self.actions.send(Command::Edit {
            junction: m.id.clone(),
            edit,
        });
        self.close_picker();
    }

    fn close_picker(&mut self) {
        self.picker = None;
        if let Some(w) = self.window.upgrade() {
            w.global::<View>().set_picker_open(false);
        }
    }

    /// The selected Junction.
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Selects Junction `id`.
    pub fn select(&mut self, id: &str) {
        // A row leaves Settings (board 25).
        if let Some(w) = self.window.upgrade() {
            w.global::<View>().set_settings_open(false);
        }
        if self.selected.as_deref() != Some(id) {
            self.selected = Some(id.to_owned());
            self.hovered = None;
            self.linking = None;
            self.close_options();
            if let Some(w) = self.window.upgrade() {
                w.global::<View>().set_renaming(false);
                w.global::<View>().set_confirming(false);
            }
            self.refresh();
        }
    }

    fn path(&self, id: &str) -> Option<String> {
        self.sidebar
            .junctions
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.path.clone())
            // The draft Cable is not in juncd.
            .filter(|p| !p.is_empty())
    }

    /// The Input or Listen output of card `i` in `column`.
    fn part(&self, column: i32, i: i32) -> Option<(String, Part)> {
        let m = self.main.as_ref()?;
        let i = usize::try_from(i).ok()?;
        let part = match column {
            0 => Part::Input(m.inputs.get(i)?.id.clone()),
            2 => Part::Listen(m.listens.get(i)?.id.clone()),
            _ => return None,
        };
        Some((m.id.clone(), part))
    }

    /// Sends `command` once its slider has rested; the newest value per
    /// key wins, and each change postpones the send.
    fn later(&mut self, key: String, command: Command) {
        self.pending.borrow_mut().insert(key, command);
        let (actions, pending) = (self.actions.clone(), self.pending.clone());
        self.timer
            .start(slint::TimerMode::SingleShot, SETTLE, move || {
                for (_, c) in std::mem::take(&mut *pending.borrow_mut()) {
                    actions.send(c);
                }
            });
    }

    fn relayout(&mut self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let view = w.global::<View>();
        let Some(m) = &self.main else { return };
        let l = layout::layout(m, self.width.get(), view.get_show_listen());
        let rows = &self.sidebar.junctions;
        let mut c = ui::canvas(m, &l, self.hovered, rows.len());
        if let Some(k) = self.linking {
            let targets = linking::targets(m, &l, k.from);
            for p in &mut c.ports {
                let Some(end) = ui::port_end(p.column, p.card, p.ch, p.side) else {
                    continue;
                };
                if end == k.from || Some(end) == k.target {
                    p.state = "snap".into();
                } else if targets.contains(&end) {
                    p.state = "valid".into();
                }
            }
            if let Some(from) = linking::at(m, &l, k.from) {
                let to = k
                    .target
                    .and_then(|t| linking::at(m, &l, t))
                    .unwrap_or(k.pointer);
                view.set_pending(ui::link_item(from, to, k.keys, false));
            }
        }
        view.set_linking(match self.linking {
            None => 0,
            Some(k) if k.keys => 2,
            Some(_) => 1,
        });
        view.set_stacked(l.stacked);
        view.set_col_x0(l.columns[0].0);
        view.set_col_x1(l.columns[1].0);
        view.set_col_x2(l.columns[2].0);
        view.set_col_w(l.columns[0].1);
        view.set_canvas_height(l.height);
        view.set_listen_heading_y(l.listen_heading.unwrap_or(0.0));
        view.set_empty_inputs(l.empty_inputs.is_some());
        view.set_empty_listens(l.empty_listens.is_some());
        view.set_empty_listens_y(l.empty_listens.map_or(0.0, |r| r.y));
        view.set_empty_h(l.empty_inputs.or(l.empty_listens).map_or(0.0, |r| r.h));
        // In place: a port keeps its element, so a drag that started on it
        // keeps the pointer while the canvas changes under it.
        update(view.get_cards(), c.cards, |m| view.set_cards(m));
        update(view.get_ports(), c.ports, |m| view.set_ports(m));
        update(view.get_links(), c.links, |m| view.set_links(m));
        if let Some(r) = self.options.and_then(|i| l.inputs.get(i)) {
            view.set_options_x(r.x);
            view.set_options_y(r.y);
            view.set_options_w(r.w);
            view.set_options_h(r.h);
        }
        self.layout = Some(l);
    }

    /// The outputs a Cable's Listen select offers (board 12), as the Add
    /// Listen picker lists them.
    fn cable_outputs(&self) -> Vec<Entry> {
        match (&self.describe, &self.selected) {
            (Some(d), Some(id)) => picker::outputs(d, id),
            _ => Vec::new(),
        }
    }

    /// Fills the Cable screen (board 12) when the selected Junction is a
    /// Cable.
    fn show_cable(&self) {
        let (Some(m), Some(d), Some(w)) = (&self.main, &self.describe, self.window.upgrade())
        else {
            return;
        };
        let view = w.global::<View>();
        let Some(to) = main_view::cable_to(d, &m.id) else {
            view.set_cable(false);
            return;
        };
        let words = w.global::<Words>();
        let outputs = self.cable_outputs();
        let listen = m.listens.first();
        let target = d
            .junctions
            .iter()
            .find(|j| j.id == m.id)
            .and_then(|j| j.listens.first())
            .map_or("default", |l| l.target.as_str());
        let index = outputs
            .iter()
            .position(|e| e.pick == Pick::Listen(target.to_owned()))
            .unwrap_or(0);
        let names: Vec<SharedString> = outputs
            .iter()
            .map(|e| {
                if e.label.is_empty() {
                    words.invoke_default_output()
                } else {
                    e.label.as_str().into()
                }
            })
            .collect();
        update(view.get_cable_outputs(), names, |m| {
            view.set_cable_outputs(m)
        });
        let from = m.inputs.first();
        view.set_cable_from(from.map_or_else(String::new, |c| c.title.clone()).into());
        view.set_cable_from_running(from.is_some_and(|c| c.state == main_view::CardState::Ok));
        view.set_cable_exclusive(from.is_some_and(|c| c.exclusive));
        view.set_cable_to(to.name.as_str().into());
        view.set_cable_to_running(to.running);
        view.set_cable_volume(i32::from(to.volume));
        view.set_cable_listen(listen.is_some_and(|l| l.enabled));
        view.set_cable_listen_index(i32::try_from(index).unwrap_or(0));
        view.set_cable_draft(m.id == DRAFT);
        view.set_cable(true);
    }

    fn cable_exclusive(&mut self, on: bool) {
        let Some(m) = &self.main else { return };
        if let Some(c) = m.inputs.first() {
            self.actions.send(Command::Edit {
                junction: m.id.clone(),
                edit: Edit::Exclusive {
                    input: c.id.clone(),
                    on,
                },
            });
        }
    }

    /// The Cable's Listen switch: its Listen output on or off, made on the
    /// output its select shows when it has none.
    fn cable_listen(&mut self, on: bool, index: i32) {
        let Some(m) = &self.main else { return };
        let edit = match m.listens.first() {
            Some(l) => Edit::Enabled(Part::Listen(l.id.clone()), on),
            None if on => {
                let outputs = self.cable_outputs();
                let Some(Pick::Listen(target)) = usize::try_from(index)
                    .ok()
                    .and_then(|i| outputs.get(i))
                    .map(|e| e.pick.clone())
                else {
                    return;
                };
                Edit::AddListen {
                    label: target.clone(),
                    target,
                }
            }
            None => return,
        };
        self.actions.send(Command::Edit {
            junction: m.id.clone(),
            edit,
        });
    }

    /// The Cable's Listen select: its Listen output to output `index`.
    fn cable_listen_target(&mut self, index: i32) {
        let Some(m) = &self.main else { return };
        let Some(l) = m.listens.first() else { return };
        let outputs = self.cable_outputs();
        let Some(Pick::Listen(target)) = usize::try_from(index)
            .ok()
            .and_then(|i| outputs.get(i))
            .map(|e| e.pick.clone())
        else {
            return;
        };
        self.actions.send(Command::Edit {
            junction: m.id.clone(),
            edit: Edit::ListenTarget {
                listen: l.id.clone(),
                target,
            },
        });
    }

    /// A link starts at a port: by the pointer (`keys` false) or Enter.
    pub fn link_start(&mut self, from: End, keys: bool) {
        let (Some(m), Some(l)) = (&self.main, &self.layout) else {
            return;
        };
        let pointer = linking::at(m, l, from).unwrap_or(Point { x: 0.0, y: 0.0 });
        let target = if keys {
            linking::targets(m, l, from).first().copied()
        } else {
            None
        };
        self.linking = Some(Linking {
            from,
            target,
            keys,
            pointer,
        });
        self.show_linking();
    }

    /// The pointer moved while dragging a link, to `p` on the canvas.
    pub fn link_drag(&mut self, p: Point) {
        let (Some(m), Some(l), Some(k)) = (&self.main, &self.layout, self.linking.as_mut()) else {
            return;
        };
        k.pointer = p;
        k.target = linking::snap(m, l, k.from, p);
        self.show_linking();
    }

    /// Chooses `to` as the target of the link made with the keys.
    pub fn link_target(&mut self, to: End) {
        if let Some(k) = self.linking.as_mut() {
            k.target = Some(to);
        }
        self.show_linking();
    }

    /// The next (`d` 1) or previous (-1) target, with the keys.
    fn link_step(&mut self, d: i32) {
        let (Some(m), Some(l), Some(k)) = (&self.main, &self.layout, self.linking.as_mut()) else {
            return;
        };
        let t = linking::targets(m, l, k.from);
        if t.is_empty() {
            return;
        }
        let now = k
            .target
            .and_then(|e| t.iter().position(|x| *x == e))
            .unwrap_or(0);
        let next = if d < 0 {
            now.saturating_sub(1)
        } else {
            (now + 1).min(t.len() - 1)
        };
        k.target = Some(t[next]);
        self.show_linking();
    }

    /// Release or Enter: links to the target, if any; either way the link
    /// being made is gone.
    pub fn link_finish(&mut self) {
        let k = self.linking.take();
        if let (Some(k), Some(m), Some(d)) = (k, &self.main, &self.describe)
            && let Some(to) = k.target
            && let Some((part, pair)) = linking::pair(m, k.from, to)
            && let Some(j) = d.junctions.iter().find(|j| j.id == m.id)
        {
            let in_use = match &part {
                Part::Input(id) => j
                    .inputs
                    .iter()
                    .find(|v| &v.id == id)
                    .map(|v| v.links.clone()),
                Part::Listen(id) => j
                    .listens
                    .iter()
                    .find(|v| &v.id == id)
                    .map(|v| v.links.clone()),
            };
            if let Some(in_use) = in_use {
                self.actions.send(Command::Edit {
                    junction: m.id.clone(),
                    edit: Edit::Link { part, pair, in_use },
                });
            }
        }
        self.show_linking();
    }

    fn link_cancel(&mut self) {
        self.linking = None;
        self.show_linking();
    }

    /// Enter on a port: starts a link, or ends the one made with the keys.
    fn link_key(&mut self, from: End) {
        if self.linking.is_some_and(|k| k.keys) {
            self.link_finish();
        } else {
            self.link_start(from, true);
        }
    }

    /// Shows the link being made, its target and what it would be.
    fn show_linking(&mut self) {
        self.relayout();
        let (Some(w), Some(m), Some(l)) = (self.window.upgrade(), &self.main, &self.layout) else {
            return;
        };
        let view = w.global::<View>();
        let Some(k) = self.linking else {
            view.set_tip_visible(false);
            return;
        };
        let Some(to) = k.target else {
            view.set_tip_visible(false);
            return;
        };
        let (card, ch, bus, listen) = match (k.from, to) {
            (End::Input { card, ch }, End::BusIn { ch: bus })
            | (End::BusIn { ch: bus }, End::Input { card, ch }) => {
                (m.inputs.get(card), ch, bus, false)
            }
            (End::BusOut { ch: bus }, End::Listen { card, ch })
            | (End::Listen { card, ch }, End::BusOut { ch: bus }) => {
                (m.listens.get(card), ch, bus, true)
            }
            _ => return,
        };
        let Some(card) = card else { return };
        let pos = card
            .channels
            .iter()
            .find(|c| c.number == ch)
            .map_or("", |c| c.position);
        let at = linking::at(m, l, to).unwrap_or(k.pointer);
        view.set_tip_owner(card.title.as_str().into());
        view.set_tip_ch(i32::from(ch));
        view.set_tip_pos(pos.into());
        view.set_tip_bus(i32::from(bus));
        view.set_tip_listen(listen);
        view.set_tip_x(at.x);
        view.set_tip_y(at.y);
        view.set_tip_visible(true);
    }

    /// Opens Input card `i`'s Options panel, or closes it when open.
    pub fn toggle_options(&mut self, i: i32) {
        let i = usize::try_from(i).ok();
        if i.is_none() || i == self.options {
            self.close_options();
            return;
        }
        self.options = i;
        self.show_options();
        self.relayout();
    }

    fn close_options(&mut self) {
        self.options = None;
        if let Some(w) = self.window.upgrade() {
            w.global::<View>().set_options_open(false);
        }
    }

    /// Fills the Options panel from the Main model; closes it when its
    /// Input is gone.
    fn show_options(&mut self) {
        let (Some(i), Some(m), Some(w)) = (self.options, &self.main, self.window.upgrade()) else {
            return;
        };
        let Some(card) = m.inputs.get(i) else {
            self.close_options();
            return;
        };
        let view = w.global::<View>();
        let words = w.global::<Words>();
        let bus: u8 = m
            .bus
            .iter()
            .map(|g| u8::try_from(g.channels.len()).unwrap_or(0))
            .sum();
        let mut options = vec![words.invoke_no_bus()];
        options.extend((1..=bus).map(|ch| words.invoke_bus_channel(i32::from(ch))));
        update(view.get_bus_options(), options, |m| view.set_bus_options(m));
        view.set_options_title(card.title.as_str().into());
        view.set_options_app(matches!(card.icon, Icon::Music | Icon::App));
        view.set_options_exclusive(card.exclusive);
        update(
            view.get_options_channels(),
            ui::option_channels(card),
            |m| {
                view.set_options_channels(m);
            },
        );
        update(view.get_options_feeds(), main_view::feeds(m, i), |m| {
            view.set_options_feeds(m);
        });
        view.set_options_index(i32::try_from(i).unwrap_or(-1));
        view.set_options_open(true);
    }

    /// The Input whose panel is open and its links in use.
    fn options_input(&self) -> Option<(String, String, Vec<[u8; 2]>)> {
        let (i, m, d) = (self.options?, self.main.as_ref()?, self.describe.as_ref()?);
        let id = m.inputs.get(i)?.id.clone();
        let j = d.junctions.iter().find(|j| j.id == m.id)?;
        let v = j.inputs.iter().find(|v| v.id == id)?;
        Some((m.id.clone(), id, v.links.clone()))
    }

    fn options_exclusive(&mut self, on: bool) {
        if let Some((junction, input, _)) = self.options_input() {
            self.actions.send(Command::Edit {
                junction,
                edit: Edit::Exclusive { input, on },
            });
        }
    }

    /// Channel `ch` feeds Bus channel `bus` (0: nothing).
    fn options_feed(&mut self, ch: i32, bus: i32) {
        let (Some((junction, input, in_use)), Ok(ch)) = (self.options_input(), u8::try_from(ch))
        else {
            return;
        };
        self.actions.send(Command::Edit {
            junction,
            edit: Edit::Feed {
                input,
                ch,
                bus: u8::try_from(bus).ok().filter(|b| *b > 0),
                in_use,
            },
        });
    }

    fn options_remove(&mut self) {
        if let Some((junction, input, _)) = self.options_input() {
            self.actions.send(Command::Edit {
                junction,
                edit: Edit::RemoveInput(input),
            });
        }
        self.close_options();
    }

    /// Shows the sidebar and the selected Junction.
    fn refresh(&mut self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let view = w.global::<View>();
        let selected = self.selected.clone();
        self.main = self
            .describe
            .as_ref()
            .zip(selected.as_deref())
            .and_then(|(d, id)| main_view::main(d, id));
        let rows = &self.sidebar.junctions;
        view.set_daemon_state(self.sidebar.daemon.as_str().into());
        view.set_junctions(ModelRc::new(VecModel::from(ui::sidebar(
            rows,
            selected.as_deref(),
        ))));
        let row = rows
            .iter()
            .position(|r| Some(r.id.as_str()) == selected.as_deref());
        view.set_selected_row(row.and_then(|i| i32::try_from(i).ok()).unwrap_or(0));
        let slots = ui::slots(rows, self.main.as_ref());
        self.levels.reset(slots.len());
        if let Some(meters) = &self.meters {
            meters.set_slots(&slots);
        }
        match &self.main {
            Some(m) => {
                view.set_title(m.name.as_str().into());
                view.set_junction_on(m.enabled);
                view.set_node(m.node.as_str().into());
                view.set_bus_channels(
                    i32::try_from(m.bus.iter().map(|g| g.channels.len()).sum::<usize>())
                        .unwrap_or(0),
                );
                view.set_apps(i32::try_from(m.apps).unwrap_or(0));
                view.set_devices(i32::try_from(m.devices).unwrap_or(0));
                view.set_off(i32::try_from(m.off).unwrap_or(0));
                view.set_offline(i32::try_from(m.offline).unwrap_or(0));
                view.set_waiting(i32::try_from(m.waiting).unwrap_or(0));
                view.set_listen_count(i32::try_from(m.listens.len()).unwrap_or(0));
                let (lines, note) = removal_words(&w.global::<Words>(), &main_view::removal(m));
                view.set_delete_lines(ModelRc::new(VecModel::from(lines)));
                view.set_delete_note(note);
                view.set_selected(m.id.as_str().into());
                self.show_cable();
            }
            None => {
                view.set_selected("".into());
                if self.picker.is_some() {
                    self.picker = None;
                    view.set_picker_open(false);
                }
            }
        }
        if self.picker.is_some() && self.main.is_some() {
            self.load_picker();
        }
        if self.options.is_some() {
            self.show_options();
        }
        if self
            .hovered
            .is_some_and(|i| self.main.as_ref().is_none_or(|m| i >= m.links.len()))
        {
            self.hovered = None;
            view.set_tip_visible(false);
        }
        self.relayout();
    }

    fn hover(&mut self, p: Option<Point>) {
        if self.linking.is_some() {
            return;
        }
        let hit = p.and_then(|p| self.layout.as_ref()?.hit(p));
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let view = w.global::<View>();
        if let (Some(i), Some(p), Some(m)) = (hit, p, &self.main) {
            let link = &m.links[i];
            let (card, ch, bus, listen) = match (link.from, link.to) {
                (End::Input { card, ch }, End::BusIn { ch: bus }) => {
                    (&m.inputs[card], ch, bus, false)
                }
                (End::BusOut { ch: bus }, End::Listen { card, ch }) => {
                    (&m.listens[card], ch, bus, true)
                }
                _ => return,
            };
            let pos = card
                .channels
                .iter()
                .find(|c| c.number == ch)
                .map_or("", |c| c.position);
            view.set_tip_owner(card.title.as_str().into());
            view.set_tip_ch(i32::from(ch));
            view.set_tip_pos(pos.into());
            view.set_tip_bus(i32::from(bus));
            view.set_tip_listen(listen);
            view.set_tip_x(p.x);
            view.set_tip_y(p.y);
        }
        view.set_tip_visible(hit.is_some());
        if hit != self.hovered {
            self.hovered = hit;
            self.relayout();
        }
    }

    /// Sets the link under the pointer (scenarios).
    pub fn hover_at(&mut self, p: Point) {
        self.hover(Some(p));
    }

    /// The layout on screen.
    pub fn layout(&self) -> Option<&Layout> {
        self.layout.as_ref()
    }

    /// The Main model on screen.
    pub fn main(&self) -> Option<&Main> {
        self.main.as_ref()
    }

    /// Removes links `links` (indexes into the Main model's links).
    fn unlink(&mut self, links: &[usize]) {
        let (Some(m), Some(d)) = (&self.main, &self.describe) else {
            return;
        };
        let Some(j) = d.junctions.iter().find(|j| j.id == m.id) else {
            return;
        };
        let mut by_owner: BTreeMap<(bool, String), Vec<[u8; 2]>> = BTreeMap::new();
        for &i in links {
            let Some(l) = m.links.get(i) else { continue };
            let listen = matches!(l.to, End::Listen { .. });
            by_owner
                .entry((listen, l.owner.clone()))
                .or_default()
                .push(l.pair);
        }
        for ((listen, owner), remove) in by_owner {
            let (part, in_use) = if listen {
                let Some(v) = j.listens.iter().find(|v| v.id == owner) else {
                    continue;
                };
                (Part::Listen(owner), v.links.clone())
            } else {
                let Some(v) = j.inputs.iter().find(|v| v.id == owner) else {
                    continue;
                };
                (Part::Input(owner), v.links.clone())
            };
            self.actions.send(Command::Edit {
                junction: m.id.clone(),
                edit: Edit::Unlink {
                    part,
                    remove,
                    in_use,
                },
            });
        }
    }

    /// Del on a port: removes every link at it.
    fn remove_port(&mut self, column: i32, card: i32, ch: i32, side: i32) {
        let Some(m) = &self.main else { return };
        let (Ok(card), Ok(ch)) = (usize::try_from(card), u8::try_from(ch)) else {
            return;
        };
        let end = match (column, side) {
            (0, _) => End::Input { card, ch },
            (1, 0) => End::BusIn { ch },
            (1, _) => End::BusOut { ch },
            _ => End::Listen { card, ch },
        };
        let links: Vec<usize> = m
            .links
            .iter()
            .enumerate()
            .filter(|(_, l)| l.from == end || l.to == end)
            .map(|(i, _)| i)
            .collect();
        self.unlink(&links);
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
}

/// Makes `app` the one [`update_current`] reaches (GUI thread).
pub fn set_current(app: &Rc<RefCell<App>>) {
    CURRENT.with(|c| *c.borrow_mut() = Some(app.clone()));
}

/// Gives the current controller juncd's new snapshot; call it on the GUI
/// thread (`slint::invoke_from_event_loop` from others).
pub fn update_current(describe: Option<Describe>) {
    let app = CURRENT.with(|c| c.borrow().clone());
    if let Some(a) = app {
        a.borrow_mut().update(describe);
    }
}

/// What switches the window between the Omarchy and the built-in theme.
type ThemeSwitch = Box<dyn Fn(bool)>;

thread_local! {
    /// Set by `main`; scenarios have none.
    static THEME: RefCell<Option<ThemeSwitch>> = const { RefCell::new(None) };
}

/// Sets what Follow Omarchy theme calls (GUI thread).
pub fn set_theme_switch(f: ThemeSwitch) {
    THEME.with(|t| *t.borrow_mut() = Some(f));
}

/// Gives the current controller what systemd says of Start at login.
pub fn service_current(enabled: Option<bool>) {
    let app = CURRENT.with(|c| c.borrow().clone());
    if let Some(a) = app {
        a.borrow_mut().set_service(enabled);
    }
}

/// Gives the current controller the system check's answer (GUI thread).
pub fn checks_current(rows: Vec<checks::Row>) {
    let app = CURRENT.with(|c| c.borrow().clone());
    if let Some(a) = app {
        a.borrow_mut().set_checks(rows);
    }
}

/// Shows the current controller again, after a language change.
pub fn refresh_current() {
    let app = CURRENT.with(|c| c.borrow().clone());
    if let Some(a) = app
        && let Ok(mut a) = a.try_borrow_mut()
    {
        a.refresh();
    }
}

/// Board 22's list of what goes with a Junction, and what stays, in the
/// UI language.
fn removal_words(words: &Words, r: &main_view::Removal) -> (Vec<SharedString>, SharedString) {
    let count = |n: usize| i32::try_from(n).unwrap_or(i32::MAX);
    let groups: Vec<SharedString> = r
        .bus
        .iter()
        .map(|&(first, stereo)| {
            if stereo {
                words.invoke_bus_pair(i32::from(first))
            } else {
                words.invoke_bus_channel(i32::from(first))
            }
        })
        .collect();
    let lines = vec![
        words.invoke_inputs_links(count(r.inputs), count(r.links)),
        words.invoke_bus_listens(
            groups
                .into_iter()
                .reduce(|a, b| words.invoke_list_comma(a, b))
                .unwrap_or_default(),
            count(r.listens),
        ),
    ];
    let names: Vec<SharedString> = r.names.iter().map(|n| n.as_str().into()).collect();
    let note = if names.is_empty() {
        SharedString::new()
    } else {
        words.invoke_keep_playing(join(words, &names), count(names.len()))
    };
    (lines, note)
}

/// "a, b and c" in the UI language.
fn join(words: &Words, items: &[SharedString]) -> SharedString {
    match items {
        [] => SharedString::new(),
        [one] => one.clone(),
        [rest @ .., before, last] => {
            let head = rest.iter().fold(None::<SharedString>, |acc, x| {
                Some(acc.map_or_else(|| x.clone(), |a| words.invoke_list_comma(a, x.clone())))
            });
            let before = head.map_or_else(
                || before.clone(),
                |h| words.invoke_list_comma(h, before.clone()),
            );
            words.invoke_list_and(before, last.clone())
        }
    }
}
