// SPDX-License-Identifier: GPL-3.0-or-later

//! The command line of `junc`: every command and flag. Shared by the binary
//! and by `cargo xtask` (the skill reference and its checks).

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "junc",
    version,
    about = "Route audio between apps and devices with Junctions"
)]
pub struct Cli {
    /// Print one JSON document instead of text.
    #[arg(long, global = true)]
    pub json: bool,
    /// Print nothing on success.
    #[arg(long, short, global = true)]
    pub quiet: bool,
    /// Do not color the output.
    #[arg(long, global = true)]
    pub no_color: bool,
    /// Language for messages (e.g. de, pt_BR).
    #[arg(long, global = true, value_name = "CODE")]
    pub lang: Option<String>,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Role {
    Input,
    Output,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// List Junctions and Cables with their status.
    List,
    /// Show the daemon, or one Junction in detail.
    Status {
        /// Junction id or name; without it, the daemon.
        name: Option<String>,
    },
    /// Create a Junction or a Cable.
    #[command(subcommand)]
    Create(Create),
    /// Rename a Junction (its id stays).
    Rename {
        /// Junction id or name (any case).
        name: String,
        /// The new name.
        new_name: String,
    },
    /// Copy a Junction.
    Duplicate {
        /// Junction id or name (any case).
        name: String,
        /// Name of the copy.
        #[arg(long = "name")]
        new_name: Option<String>,
    },
    /// Delete a Junction and its routing.
    Delete {
        /// Junction id or name (any case).
        name: String,
        /// Do not ask.
        #[arg(long)]
        yes: bool,
    },
    /// Turn a Junction on.
    Enable {
        /// Junction id or name (any case).
        name: String,
    },
    /// Turn a Junction off.
    Disable {
        /// Junction id or name (any case).
        name: String,
    },
    /// Turn a Junction on if it is off, and off if it is on.
    Toggle {
        /// Junction id or name (any case).
        name: String,
    },
    /// Turn a Junction on and make it the default input or output.
    Switch {
        /// Junction id or name (any case).
        name: String,
        #[arg(long = "as")]
        role: Option<Role>,
    },
    /// Make a Junction the default input or output, or stop that.
    Default {
        /// Junction id or name (any case).
        name: String,
        #[arg(long = "as", required_unless_present = "clear")]
        role: Option<Role>,
        #[arg(long, conflicts_with = "role")]
        clear: bool,
    },
    /// Set the volume of a Junction, one of its Inputs, or a Listen output.
    Volume {
        /// Junction id or name (any case).
        name: String,
        /// `PERCENT`, or `INPUT PERCENT`.
        #[arg(num_args = 1..=2, required = true)]
        args: Vec<String>,
        /// Set the volume of this Listen output.
        #[arg(long, value_name = "ID")]
        listen: Option<String>,
    },
    /// Add, remove, enable or disable Inputs.
    #[command(subcommand)]
    Input(InputCmd),
    /// Add or remove Bus channels.
    #[command(subcommand)]
    Bus(BusCmd),
    /// Link an Input channel to a Bus channel (`mic:1 2`), or a Bus channel
    /// to a Listen output channel (`1 speakers:2`).
    Link {
        /// Junction id or name (any case).
        name: String,
        /// `INPUT:CH` (an Input channel) or a Bus channel number.
        from: String,
        /// A Bus channel number, or `LISTEN:CH` (a Listen output channel).
        to: String,
    },
    /// Remove a link (same arguments as `link`).
    Unlink {
        /// Junction id or name (any case).
        name: String,
        /// `INPUT:CH` (an Input channel) or a Bus channel number.
        from: String,
        /// A Bus channel number, or `LISTEN:CH` (a Listen output channel).
        to: String,
    },
    /// Add or remove Listen outputs.
    #[command(subcommand)]
    Listen(ListenCmd),
    /// Everything an agent needs in one JSON document.
    Describe,
    /// Print the JSON Schema of junctions.toml.
    Schema {
        /// Config schema version.
        #[arg(long = "version", default_value_t = 1)]
        schema_version: u32,
    },
    /// Find, check or edit the config file.
    #[command(subcommand)]
    Config(ConfigCmd),
    /// Apply a whole config file (`-` reads stdin).
    Apply {
        /// A whole config file, or `-` for stdin.
        file: PathBuf,
        /// Show what would change; change nothing.
        #[arg(long)]
        dry_run: bool,
        /// Refuse if the config changed since this revision.
        #[arg(long, value_name = "N")]
        expect_revision: Option<u64>,
    },
    /// Check PipeWire, WirePlumber, pipewire-pulse, JACK, the clock and juncd.
    Doctor,
    /// Start juncd at login, or not (`systemctl --user` on juncd.service).
    #[command(subcommand)]
    Service(ServiceCmd),
    /// Make the JUNC skill visible to your coding agents, or not.
    #[command(subcommand)]
    Skill(SkillCmd),
}

#[derive(Debug, Subcommand)]
pub enum SkillCmd {
    /// Link the skill into every agent skills folder you have.
    Link,
    /// Remove JUNC's skill links (your own `junc` folders stay).
    Unlink,
    /// Show which agent folders have the skill.
    Status,
}

#[derive(Debug, Subcommand)]
pub enum ServiceCmd {
    /// Start juncd at login, and now.
    Enable,
    /// Stop starting juncd at login, and stop it now.
    Disable,
    /// Whether juncd starts at login and runs.
    Status,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCmd {
    /// Print where the config file is.
    Path,
    /// Check a config file without applying it (default: the one in use).
    Validate { file: Option<PathBuf> },
    /// Open the config in $VISUAL or $EDITOR, then check it.
    Edit,
}

#[derive(Debug, Subcommand)]
pub enum Create {
    /// An empty Junction.
    Junction {
        /// Junction id or name (any case).
        name: String,
    },
    /// One app straight into another.
    Cable {
        /// Binary of the app that plays.
        from_app: String,
        /// Binary of the app that listens.
        to_app: String,
        #[arg(long)]
        name: Option<String>,
    },
}

#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
pub struct SourceArgs {
    /// An app, by binary name.
    #[arg(long, value_name = "BINARY")]
    pub app: Option<String>,
    /// A hardware input, by node name.
    #[arg(long, value_name = "NODE")]
    pub device: Option<String>,
    /// What an output plays, by sink node name.
    #[arg(long, value_name = "SINK")]
    pub monitor: Option<String>,
    /// Another Junction.
    #[arg(long, value_name = "NAME")]
    pub junction: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum InputCmd {
    /// Add an Input.
    Add {
        /// Junction id or name (any case).
        name: String,
        #[command(flatten)]
        source: SourceArgs,
        /// Take the app away from the default output while captured.
        #[arg(long)]
        exclusive: bool,
        /// Switch the device's card to Pro Audio for all its channels, now
        /// and whenever it is plugged in (see `devices` in `junc describe`).
        #[arg(long, requires = "device")]
        pro_audio: bool,
        /// Id of the new Input.
        #[arg(long)]
        id: Option<String>,
    },
    /// Remove an Input.
    Remove {
        /// Junction id or name (any case).
        name: String,
        /// Input id (see `junc status NAME`).
        input: String,
    },
    /// Turn an Input on.
    Enable {
        /// Junction id or name (any case).
        name: String,
        /// Input id (see `junc status NAME`).
        input: String,
    },
    /// Turn an Input off.
    Disable {
        /// Junction id or name (any case).
        name: String,
        /// Input id (see `junc status NAME`).
        input: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum BusCmd {
    /// Add channels: a stereo pair, or one with --mono.
    Add {
        /// Junction id or name (any case).
        name: String,
        #[arg(long)]
        mono: bool,
        /// How many pairs (or channels with --mono).
        #[arg(long, default_value_t = 1)]
        count: u8,
    },
    /// Remove one channel; links to later channels move down.
    Remove {
        /// Junction id or name (any case).
        name: String,
        /// The Bus channel to remove (1-based).
        channel: u8,
    },
}

#[derive(Debug, Subcommand)]
pub enum ListenCmd {
    /// Hear the Bus on an output (`default` follows the system output).
    Add {
        /// Junction id or name (any case).
        name: String,
        /// `default`, or an output node name.
        output: String,
        #[arg(long)]
        id: Option<String>,
    },
    /// Stop hearing the Bus on an output.
    Remove {
        /// Junction id or name (any case).
        name: String,
        /// Listen output id.
        id: String,
    },
}
