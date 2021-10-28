// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tracing subscribers to send data to internal logs and to format data.

use crate::output::OutputOpts;
use colored::Colorize;
use std::fmt::{self, Write};
use tracing::{field::Field, level_filters::LevelFilter, Event, Level, Subscriber};
use tracing_subscriber::{
    field::Visit,
    filter::{FilterFn, Targets},
    fmt::{format::Writer, FmtContext, FormatEvent, FormatFields},
    prelude::*,
    registry::LookupSpan,
    Layer, Registry,
};

static HASP_LOG_ENV: &str = "HASP_LOG";
static MESSAGE_FIELD: &str = "message";

impl OutputOpts {
    pub(super) fn make_subscriber(&self) {
        let registry = tracing_subscriber::registry();

        let level_str = std::env::var_os(HASP_LOG_ENV).unwrap_or_default();
        let level_str = level_str
            .into_string()
            .unwrap_or_else(|_| panic!("{} is not UTF-8", HASP_LOG_ENV));
        // If the level string is empty, use the standard level filter instead.
        let targets = if level_str.is_empty() {
            let level_filter = if self.quiet {
                LevelFilter::ERROR
            } else if self.verbose == 0 {
                LevelFilter::INFO
            } else {
                LevelFilter::DEBUG
            };
            Targets::new().with_default(level_filter)
        } else {
            level_str.parse().expect("unable to parse HASP_LOG")
        };

        // Environment-based and command-line based logging.

        let fmt_layer: Box<dyn Layer<Registry> + Send + Sync> = match self.verbose {
            0..=1 => {
                let output_layer = tracing_subscriber::fmt::layer()
                    .event_format(OutputFormatter)
                    .with_writer(std::io::stderr)
                    .with_filter(FilterFn::new(|metadata| {
                        metadata.is_event() && metadata.target().starts_with("hasp::output::")
                    }));
                let alt_layer = tracing_subscriber::fmt::layer()
                    .event_format(AltOutputFormatter)
                    .with_writer(std::io::stderr)
                    .with_filter(FilterFn::new(|metadata| {
                        metadata.is_event() && metadata.target().starts_with("hasp::alt_output::")
                    }));

                let combined = output_layer.and_then(alt_layer).with_filter(targets);
                Box::new(combined)
            }
            2 => {
                // Output all events through the event formatter.
                let fmt_layer = tracing_subscriber::fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_filter(targets);
                Box::new(fmt_layer)
            }
            _ => {
                // Output all events through the pretty formatter.
                let fmt_layer = tracing_subscriber::fmt::layer()
                    .with_writer(std::io::stderr)
                    .pretty()
                    .with_filter(targets);
                Box::new(fmt_layer)
            }
        };

        registry.with(fmt_layer).init();
    }
}

struct OutputFormatter;

impl<S, N> FormatEvent<S, N> for OutputFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut f: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let kind = OutputKind::from_target(event.metadata().target());
        let level = *event.metadata().level();

        let mut visitor = MessageVisitor {
            kind,
            level,
            writer: &mut f,
        };
        event.record(&mut visitor);

        writeln!(f)?;

        Ok(())
    }
}

#[derive(Debug)]
enum OutputKind {
    Working,
    Recording,
    Informational,
    Standard,
}

impl OutputKind {
    fn from_target(target: &str) -> Self {
        if target.starts_with("hasp::output::working::") {
            Self::Working
        } else if target.starts_with("hasp::output::recording::") {
            Self::Recording
        } else if target.starts_with("hasp::output::informational::") {
            Self::Informational
        } else {
            Self::Standard
        }
    }
}

struct MessageVisitor<'writer, 'a> {
    kind: OutputKind,
    level: Level,
    writer: &'a mut Writer<'writer>,
}

impl<'writer, 'a> Visit for MessageVisitor<'writer, 'a> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == MESSAGE_FIELD {
            let message = format!("{:?}", value);
            let (header, text) = message.split_once(' ').unwrap_or(("", &message));

            let header = if self.level == Level::ERROR {
                header.bold().red()
            } else if self.level == Level::WARN {
                header.bold().yellow()
            } else {
                match self.kind {
                    OutputKind::Recording => header.bold().yellow(),
                    OutputKind::Working => header.bold().blue(),
                    OutputKind::Informational => header.bold().purple(),
                    OutputKind::Standard => header.bold().green(),
                }
            };
            // This uses the same alignment as Cargo itself.
            let _ = write!(self.writer, "{:>12} ", header);
            // Print out the first newline non-indented.
            match text.split_once('\n') {
                Some((first_line, later)) => {
                    let _ = writeln!(self.writer, "{}", first_line);
                    let _ = write!(
                        indenter::indented(self.writer).with_str("             "),
                        "{}",
                        later
                    );
                }
                None => {
                    let _ = write!(self.writer, "{}", text);
                }
            }
        }
    }
}

struct AltOutputFormatter;

impl<S, N> FormatEvent<S, N> for AltOutputFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut f: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        let level = *event.metadata().level();

        if level == Level::ERROR {
            write!(f, "{} ", "error:".bold().red())?;
        } else if level == Level::WARN {
            write!(f, "{} ", "warning:".bold().yellow())?;
        } else if level == Level::INFO {
            write!(f, "{} ", "info:".bold().blue())?;
        } else if level == Level::DEBUG {
            write!(f, "{} ", "debug:".bold())?;
        }

        let mut visitor = AltMessageVisitor { writer: &mut f };
        event.record(&mut visitor);

        writeln!(f)?;

        Ok(())
    }
}

struct AltMessageVisitor<'writer, 'a> {
    writer: &'a mut Writer<'writer>,
}

impl<'writer, 'a> Visit for AltMessageVisitor<'writer, 'a> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == MESSAGE_FIELD {
            let _ = write!(self.writer, "{:?}", value);
        }
    }
}
