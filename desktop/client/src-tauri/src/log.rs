//! Tracing subscriber for the desktop application (stderr).

use std::fmt;
use std::io;

use nu_ansi_term::{Color, Style};
use tracing::{Event, Level};
use tracing_subscriber::fmt::format::{FormatFields, Writer};
use tracing_subscriber::fmt::time::{FormatTime, LocalTime};
use tracing_subscriber::fmt::{FmtContext, FormatEvent};
use tracing_subscriber::registry::LookupSpan;

/// Install a global tracing subscriber writing to stderr.
///
/// Respects `RUST_LOG` when set; otherwise defaults to `info`.
/// Safe to call more than once: subsequent calls are ignored.
pub fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let timer = LocalTime::new(
        time::format_description::parse(
            "[year][month padding:zero][day padding:zero] [hour]:[minute]:[second].[subsecond digits:3]",
        )
        .expect("valid log timestamp format"),
    );

    let _ = tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_ansi(true)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .event_format(MorayEventFormat { timer })
        .try_init();
}

struct MorayEventFormat<T> {
    timer: LocalTime<T>,
}

impl<S, N, T> FormatEvent<S, N> for MorayEventFormat<T>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
    LocalTime<T>: FormatTime,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let metadata = event.metadata();

        self.timer.format_time(&mut writer)?;
        write_level(&mut writer, metadata.level())?;
        write_target(&mut writer, metadata.target())?;
        ctx.field_format().format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

fn write_level(writer: &mut Writer<'_>, level: &Level) -> fmt::Result {
    if writer.has_ansi_escapes() {
        let colored = match *level {
            Level::TRACE => Color::Purple.paint("TRACE"),
            Level::DEBUG => Color::Blue.paint("DEBUG"),
            Level::INFO => Color::Green.paint(" INFO"),
            Level::WARN => Color::Yellow.paint(" WARN"),
            Level::ERROR => Color::Red.paint("ERROR"),
        };
        write!(writer, " {} ", colored)
    } else {
        write!(writer, " {:5} ", level)
    }
}

const TARGET_WIDTH: usize = 20;
const TARGET_ELLIPSIS: &str = "...";

fn write_target(writer: &mut Writer<'_>, target: &str) -> fmt::Result {
    let padded = pad_target(&shorten_target(target));
    if writer.has_ansi_escapes() {
        write!(writer, "{} ", Style::new().dimmed().paint(&padded))
    } else {
        write!(writer, "{padded} ")
    }
}

fn shorten_target(target: &str) -> String {
    let parts: Vec<&str> = target.split("::").collect();
    match parts.len() {
        0 => String::new(),
        1 | 2 => target.to_string(),
        n => parts[n - 2..].join("::"),
    }
}

fn pad_target(target: &str) -> String {
    if target.len() > TARGET_WIDTH {
        let visible = TARGET_WIDTH - TARGET_ELLIPSIS.len();
        format!(
            "{TARGET_ELLIPSIS}{}",
            &target[target.len().saturating_sub(visible)..]
        )
    } else {
        format!("{target:>TARGET_WIDTH$}")
    }
}

#[cfg(test)]
mod tests {
    use super::{pad_target, shorten_target, TARGET_WIDTH};

    #[test]
    fn shorten_target_keeps_last_two_segments() {
        assert_eq!(
            shorten_target("moray_extensions::channels::wecom::channel"),
            "wecom::channel",
        );
    }

    #[test]
    fn shorten_target_unchanged_when_at_most_two_segments() {
        assert_eq!(shorten_target("wecom::channel"), "wecom::channel");
        assert_eq!(shorten_target("agent"), "agent");
    }

    #[test]
    fn pad_target_right_aligns() {
        assert_eq!(
            pad_target("wecom::channel"),
            format!("{:>TARGET_WIDTH$}", "wecom::channel"),
        );
        assert_eq!(pad_target("agent"), format!("{:>TARGET_WIDTH$}", "agent"));
    }

    #[test]
    fn pad_target_clips_from_front_when_too_long() {
        assert_eq!(
            pad_target(&shorten_target(
                "foo::bar::very_long_module_name_that_exceeds_width"
            )),
            "...hat_exceeds_width",
        );
    }

    #[test]
    fn pad_target_clips_from_front_preserving_tail() {
        assert_eq!(pad_target("extensions::wecom::channel"), "...s::wecom::channel");
    }
}
