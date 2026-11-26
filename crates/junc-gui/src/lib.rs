// SPDX-License-Identifier: GPL-3.0-or-later

//! JUNC graphical interface (Slint), docs/architecture.md §7. The logic
//! lives here, as a library, so it is tested without a display; `main.rs`
//! only starts it.

// deny, not forbid: the code Slint generates (ui.rs) needs `unsafe`;
// JUNC's own code has none.
#![deny(unsafe_code)]

pub mod daemon;
pub mod main_view;
pub mod meter_service;
pub mod meters;
#[cfg(feature = "scenarios")]
pub mod scenario;
#[cfg(feature = "scenarios")]
pub mod snapshot;
pub mod state;
pub mod theme;
pub mod theme_service;
pub mod ui;
pub mod visibility;
