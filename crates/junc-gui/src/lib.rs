// SPDX-License-Identifier: GPL-3.0-or-later

//! JUNC graphical interface (Slint), docs/architecture.md §7. The logic
//! lives here, as a library, so it is tested without a display; `main.rs`
//! only starts it.

#![forbid(unsafe_code)]

pub mod theme;
