// SPDX-License-Identifier: GPL-3.0-or-later

//! Platform-agnostic core of JUNC: model, configuration, graph view,
//! identity matching and the reconcile planner. No PipeWire, D-Bus or UI
//! dependency is allowed here (see docs/architecture.md §2).

#![forbid(unsafe_code)]

pub mod backend;
pub mod bluetooth;
pub mod config;
pub mod defaults;
pub mod diag;
pub mod graph;
pub mod import;
pub mod matcher;
pub mod migrate;
pub mod model;
pub mod plan;
pub mod proaudio;
pub mod reconcile;
pub mod schema;
pub mod store;
