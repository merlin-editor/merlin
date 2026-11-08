//! The Output panel: a channel for merlin's own messages and one for each
//! plugin that logs, with a choice of channel and of how much to show.
//! Plugins write to theirs with `log`, and their programs' stderr lands
//! there too, so a notice is only for what needs the user.

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use super::views::OUTPUT;
use super::{Editor, Effect, Side, Source};
use crate::panel::{LogRow, Node, Tone};

/// Most lines a channel keeps; older ones are dropped.
const CHANNEL_LINES: usize = 1000;

/// merlin's own channel.
pub(crate) const MERLIN: &str = "merlin";

/// How serious a line is, least first. `Stderr` is what a plugin's
/// program printed there, which may be anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Stderr,
}

impl LogLevel {
    fn label(self) -> &'static str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
            LogLevel::Stderr => "stderr",
        }
    }

    fn tone(self) -> Tone {
        match self {
            LogLevel::Trace | LogLevel::Debug => Tone::Muted,
            LogLevel::Info => Tone::Strong,
            LogLevel::Warn => Tone::Warning,
            LogLevel::Error => Tone::Error,
            LogLevel::Stderr => Tone::Normal,
        }
    }

    /// The setting's name for it, as in `notices` and `log-level`.
    pub fn parse(s: &str) -> Option<LogLevel> {
        Some(match s {
            "trace" => LogLevel::Trace,
            "debug" => LogLevel::Debug,
            "info" => LogLevel::Info,
            "warn" | "warning" => LogLevel::Warn,
            "error" => LogLevel::Error,
            _ => return None,
        })
    }
}

/// The least a line must be to show. Stderr shows from Info up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShowLevel {
    All,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

const SHOW_LEVELS: [(ShowLevel, &str); 5] = [
    (ShowLevel::All, "All"),
    (ShowLevel::Debug, "Debug and up"),
    (ShowLevel::Info, "Info and up"),
    (ShowLevel::Warn, "Warnings and errors"),
    (ShowLevel::Error, "Errors only"),
];

impl ShowLevel {
    fn shows(self, level: LogLevel) -> bool {
        let level = if level == LogLevel::Stderr {
            LogLevel::Info
        } else {
            level
        };
        level
            >= match self {
                ShowLevel::All => LogLevel::Trace,
                ShowLevel::Debug => LogLevel::Debug,
                ShowLevel::Info => LogLevel::Info,
                ShowLevel::Warn => LogLevel::Warn,
                ShowLevel::Error => LogLevel::Error,
            }
    }

    fn label(self) -> &'static str {
        SHOW_LEVELS
            .iter()
            .find(|(l, _)| *l == self)
            .map_or("", |(_, s)| s)
    }
}

#[derive(Clone, Debug)]
struct Line {
    /// Milliseconds since 1970, UTC.
    at: u64,
    level: LogLevel,
    text: String,
}

#[derive(Debug)]
struct Channel {
    /// [`MERLIN`] or a plugin's name.
    name: String,
    lines: VecDeque<Line>,
    /// An error came in since the channel was last looked at.
    unseen_error: bool,
}

/// Which of the bar's lists is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputList {
    Channels,
    Levels,
}

#[derive(Debug)]
pub(crate) struct Output {
    channels: Vec<Channel>,
    /// The channel showing.
    current: String,
    show: ShowLevel,
    filter: String,
    open: Option<OutputList>,
    /// The local time's offset from UTC, in seconds, for the times shown.
    pub(crate) utc_offset: i64,
    /// Bumped by every change, so the view is rebuilt only then.
    version: u64,
    built: Option<u64>,
}

impl Default for Output {
    fn default() -> Self {
        Self {
            channels: vec![Channel {
                name: MERLIN.into(),
                lines: VecDeque::new(),
                unseen_error: false,
            }],
            current: MERLIN.into(),
            show: ShowLevel::default(),
            filter: String::new(),
            open: None,
            utc_offset: 0,
            version: 0,
            built: None,
        }
    }
}

/// The Output panel's bar, as the window draws it over the panel's tabs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputBarView {
    /// The channel showing: its title and its tile.
    pub channel: String,
    pub mark: Option<(String, crate::style::Hue)>,
    pub level: String,
    pub open: Option<OutputList>,
    /// The open list's rows.
    pub rows: Vec<OutputRowView>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputRowView {
    pub label: String,
    pub mark: Option<(String, crate::style::Hue)>,
    /// Lines in the channel, for a channel row.
    pub count: Option<usize>,
    pub unseen_error: bool,
    pub on: bool,
    /// Where a heading goes above this row: "Plugins".
    pub heading: Option<String>,
}

impl Output {
    fn channel_mut(&mut self, name: &str) -> &mut Channel {
        let i = match self.channels.iter().position(|c| c.name == name) {
            Some(i) => i,
            None => {
                self.channels.push(Channel {
                    name: name.into(),
                    lines: VecDeque::new(),
                    unseen_error: false,
                });
                // merlin first, then plugins by name.
                self.channels[1..].sort_by(|a, b| a.name.cmp(&b.name));
                self.channels.iter().position(|c| c.name == name).unwrap()
            }
        };
        &mut self.channels[i]
    }

    pub(crate) fn push(&mut self, channel: &str, level: LogLevel, text: &str) {
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        let c = self.channel_mut(channel);
        // A multi-line message is a line each, at the same time.
        for line in text.lines() {
            if c.lines.len() == CHANNEL_LINES {
                c.lines.pop_front();
            }
            c.lines.push_back(Line {
                at,
                level,
                text: line.to_string(),
            });
        }
        if level == LogLevel::Error {
            c.unseen_error = true;
        }
        self.version += 1;
    }

    /// `HH:MM:SS.mmm` in local time.
    fn time(&self, at: u64) -> String {
        let local = at as i64 + self.utc_offset * 1000;
        let ms = local.rem_euclid(1000);
        let secs = local.div_euclid(1000).rem_euclid(86_400);
        format!(
            "{:02}:{:02}:{:02}.{ms:03}",
            secs / 3600,
            secs / 60 % 60,
            secs % 60
        )
    }
}

impl Editor {
    /// A line in a channel of the Output panel.
    pub(crate) fn log_line(&mut self, channel: &str, level: LogLevel, text: &str) {
        self.output.push(channel, level, text);
    }

    /// Sets the local time's offset from UTC, for the Output panel's times.
    pub fn set_utc_offset(&mut self, seconds: i64) {
        self.output.utc_offset = seconds;
        self.output.version += 1;
    }

    fn output_showing(&self) -> bool {
        self.bar(Side::Bottom).visible
            && self.active_panel_name(Side::Bottom).as_deref() == Some(OUTPUT)
    }

    /// Shows a channel in the Output panel: merlin's, or a plugin's.
    pub(crate) fn show_output(&mut self, channel: &str) {
        self.output.channel_mut(channel);
        self.output.current = channel.into();
        self.output.open = None;
        self.output.version += 1;
        self.show_bottom(OUTPUT);
    }

    /// Rebuilds the Output view if something changed while it shows.
    pub(crate) fn build_output(&mut self) {
        if !self.output_showing() {
            return;
        }
        let current = self.output.current.clone();
        if let Some(c) = self.output.channels.iter_mut().find(|c| c.name == current)
            && c.unseen_error
        {
            c.unseen_error = false;
            self.output.version += 1;
        }
        let typed = self
            .inputs
            .get(&(OUTPUT.to_string(), "filter".to_string()))
            .cloned()
            .unwrap_or_default();
        self.output_filter(&typed);
        if self.output.built == Some(self.output.version) {
            return;
        }
        self.output.built = Some(self.output.version);
        let nodes = self.output_nodes();
        self.set_view(OUTPUT, nodes);
    }

    fn output_nodes(&self) -> Vec<Node> {
        let o = &self.output;
        let Some(c) = o.channels.iter().find(|c| c.name == o.current) else {
            return Vec::new();
        };
        let filter = o.filter.to_lowercase();
        let rows: Vec<Node> = c
            .lines
            .iter()
            .filter(|l| o.show.shows(l.level))
            .filter(|l| filter.is_empty() || l.text.to_lowercase().contains(&filter))
            .map(|l| {
                Node::Log(LogRow {
                    time: o.time(l.at),
                    level: l.level.label().into(),
                    tone: l.level.tone(),
                    text: l.text.clone(),
                })
            })
            .collect();
        let mut nodes = rows;
        if nodes.is_empty() {
            let why = if c.lines.is_empty() {
                format!("Nothing from {} yet.", self.channel_title(&c.name))
            } else if !filter.is_empty() {
                "No lines match the filter.".to_string()
            } else {
                format!("No lines at {}.", o.show.label().to_lowercase())
            };
            nodes.push(Node::Styled(why, Tone::Muted));
        }
        nodes.push(Node::Footer);
        nodes.push(Node::Input {
            id: "filter".into(),
            placeholder: "Filter".into(),
            secret: false,
            multiline: false,
        });
        nodes
    }

    fn channel_title(&self, name: &str) -> String {
        if name == MERLIN {
            return "merlin".into();
        }
        self.plugin_list
            .iter()
            .find(|p| p.name == name)
            .map_or_else(|| name.to_string(), |p| p.title.clone())
    }

    fn channel_mark(&self, name: &str) -> Option<(String, crate::style::Hue)> {
        self.plugin_list
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.mark.clone())
    }

    /// The bar over the Output panel, while it shows.
    pub(crate) fn output_bar(&self) -> Option<OutputBarView> {
        if !self.output_showing() {
            return None;
        }
        let o = &self.output;
        let rows = match o.open {
            None => Vec::new(),
            Some(OutputList::Channels) => o
                .channels
                .iter()
                .enumerate()
                .map(|(i, c)| OutputRowView {
                    label: self.channel_title(&c.name),
                    mark: self.channel_mark(&c.name),
                    count: Some(c.lines.len()),
                    unseen_error: c.unseen_error,
                    on: c.name == o.current,
                    heading: (i == 1).then(|| "Plugins".into()),
                })
                .collect(),
            Some(OutputList::Levels) => SHOW_LEVELS
                .iter()
                .map(|(l, label)| OutputRowView {
                    label: (*label).into(),
                    mark: None,
                    count: None,
                    unseen_error: false,
                    on: *l == o.show,
                    heading: None,
                })
                .collect(),
        };
        Some(OutputBarView {
            channel: self.channel_title(&o.current),
            mark: self.channel_mark(&o.current),
            level: o.show.label().into(),
            open: o.open,
            rows,
        })
    }

    /// A click on the Output bar: a list opened or closed, a row of it
    /// picked, or Clear.
    pub(crate) fn output_click(&mut self, click: OutputClick) {
        let o = &mut self.output;
        match click {
            OutputClick::Open(list) => {
                o.open = if o.open == Some(list) {
                    None
                } else {
                    Some(list)
                };
            }
            OutputClick::Close => o.open = None,
            OutputClick::Row(i) => {
                match o.open {
                    Some(OutputList::Channels) => {
                        if let Some(c) = o.channels.get(i) {
                            o.current = c.name.clone();
                        }
                    }
                    Some(OutputList::Levels) => {
                        if let Some((l, _)) = SHOW_LEVELS.get(i) {
                            o.show = *l;
                        }
                    }
                    None => {}
                }
                o.open = None;
            }
            OutputClick::Clear => {
                let current = o.current.clone();
                o.channel_mut(&current).lines.clear();
            }
        }
        self.output.version += 1;
    }

    /// The Output panel's filter box, as typed.
    fn output_filter(&mut self, text: &str) {
        if self.output.filter != text {
            self.output.filter = text.to_string();
            self.output.version += 1;
        }
    }

    /// Asks how much a plugin's notices should show: all of them, errors
    /// only, or none, with the rest in its channel.
    pub(crate) fn pick_notices(&mut self, plugin: &str) {
        let title = self.channel_title(plugin);
        let items = [
            ("all", "All notices"),
            ("errors", "Errors only"),
            ("none", "None: log them only"),
        ]
        .map(|(id, label)| (format!("{plugin}:{id}"), label.to_string()))
        .to_vec();
        self.palette = Some(super::Palette::new(
            Source::Picker {
                plugin: OUTPUT.into(),
                id: "notices".into(),
            },
            Some(format!("Notices from {title}")),
            items,
        ));
        self.focus = super::Focus::Palette;
    }

    /// A plugin's notice: its text in its channel too, its buttons, Open
    /// Output, and a link to quieten it.
    pub(crate) fn plugin_notice(
        &mut self,
        plugin: &str,
        level: LogLevel,
        text: String,
        buttons: Vec<crate::panel::Button>,
    ) {
        self.log_line(plugin, level, &text);
        let error = level >= LogLevel::Error;
        self.message = Some(super::Message {
            text,
            error,
            done: false,
            warning: level == LogLevel::Warn,
            actions: Vec::new(),
            link: Some((
                format!("Notices from {}…", self.channel_title(plugin)),
                format!("output.notices:{plugin}"),
            )),
        });
        // The plugin's own buttons last, so its main one is the notice's.
        self.add_action("Open Output", &format!("output.show:{plugin}"));
        for b in buttons {
            self.add_action(&b.label, &format!("plugin.click:{plugin}:{}", b.id));
        }
    }

    /// The Output panel's commands, and a notice's button for a plugin.
    pub(crate) fn output_command(&mut self, id: &str, fx: &mut Vec<Effect>) -> bool {
        if let Some(plugin) = id.strip_prefix("output.show:") {
            self.show_output(plugin);
        } else if let Some(plugin) = id.strip_prefix("output.notices:") {
            self.pick_notices(plugin);
        } else if let Some((plugin, button)) = id
            .strip_prefix("plugin.click:")
            .and_then(|r| r.split_once(':'))
        {
            // `command:<id>` runs one of the plugin's commands.
            if let Some(command) = button.strip_prefix("command:") {
                self.run(&format!("{plugin}.{command}"), fx);
                return true;
            }
            fx.push(Effect::PluginClick {
                plugin: plugin.into(),
                button: button.into(),
            });
        } else if id == "output.clear" {
            self.output_click(OutputClick::Clear);
        } else {
            return false;
        }
        true
    }

    /// A choice from [`Self::pick_notices`]: saved as the plugin's
    /// `notices` setting.
    pub(crate) fn notices_picked(&mut self, item: &str, fx: &mut Vec<Effect>) {
        let Some((plugin, value)) = item.rsplit_once(':') else {
            return;
        };
        fx.push(Effect::SetSetting {
            key: format!("plugins.{plugin}.notices"),
            value: format!("\"{value}\""),
        });
        let title = self.channel_title(plugin);
        let what = match value {
            "all" => "all its notices",
            "errors" => "only its errors",
            _ => "no notices; the rest are in Output",
        };
        self.notify_done(format!("{title} will show {what}"));
    }
}

/// What was clicked on the Output bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputClick {
    Open(OutputList),
    Close,
    Row(usize),
    Clear,
}
