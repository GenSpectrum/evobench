//! Experimental attempt at a table printing abstraction that can both
//! print to a terminal in nice human-readable format (with spaces for
//! padding, and ANSI sequences for formatting), as well as in CSV
//! (with tabs) format.

//! Does not currently escape anything in the fields, just uses
//! `Display` and prints that directly. Thus is not safe if the type
//! can print tabs or newlines (or on the terminal even spaces could
//! make it ambiguous).

use std::{
    io::{BufWriter, IsTerminal, Write},
    os::unix::ffi::OsStrExt,
};

use crate::{
    output_table::{CellValue, OutputStyle, OutputTable, OutputTableTitle, Row},
    utillib::get_terminal_width::get_terminal_width,
};
use anyhow::{Result, anyhow, bail};
use lazy_static::lazy_static;
use strum_macros::EnumString;
use yansi::{Color, Paint, Style};

impl From<OutputStyle> for Style {
    fn from(value: OutputStyle) -> Self {
        let OutputStyle {
            faded,
            bold,
            italic,
            color,
            font_size: _,
        } = value;
        let mut style = Style::new();
        if faded {
            // Note: needs `TERM=xterm-256color`
            // for `watch --color` to not turn
            // this color to black!
            style = style.bright_black()
        }
        if bold {
            style = style.bold()
        }
        if italic {
            style = style.italic()
        }
        if let Some(col) = color {
            // Note: in spite of `TERM=xterm-256color`, `watch
            // --color` still only supports system colors
            // 0..14!  (Can still not use `.rgb(10, 70, 140)`
            // nor `.fg(Color::Fixed(30))`, and watch 4.0.2
            // does not support `TERM=xterm-truecolor`.)
            style = style.fg(Color::Fixed(col))
        }

        style
    }
}

lazy_static! {
    static ref UNICODE_IS_FINE: bool = (|| -> Option<bool> {
        let term = std::env::var_os("TERM")?;
        let lang = std::env::var_os("LANG")?;
        let lang = lang.to_str()?;
        Some(term.as_bytes().starts_with(b"xterm") && lang.contains("UTF-8"))
    })()
    .unwrap_or(false);
}

#[derive(Debug, EnumString, PartialEq, Clone, Copy)]
#[strum(serialize_all = "kebab_case")]
pub enum ColorOpt {
    Auto,
    Always,
    Never,
}

impl ColorOpt {
    pub fn want_color(self, detected_terminal: bool) -> bool {
        match self {
            ColorOpt::Auto => detected_terminal,
            ColorOpt::Always => true,
            ColorOpt::Never => false,
        }
    }
}

#[derive(Debug, clap::Args, Clone)]
pub struct TerminalTableOpts {
    /// Show the table as CSV (with '\t' as separator) instead of
    /// human-readable
    #[clap(long)]
    tsv: bool,

    /// Whether to use ANSI codes to format human-readable output on
    /// terminals (auto, always, never)
    #[clap(long, default_value = "auto")]
    color: ColorOpt,
}

impl TerminalTableOpts {
    pub fn want_color(&self, detected_terminal: bool) -> bool {
        let Self { tsv, color } = self;
        if *tsv {
            false
        } else {
            color.want_color(detected_terminal)
        }
    }
}

struct TerminalTableSettings {
    widths: Vec<usize>,
    padding: String,
    is_terminal: bool,
}

/// Capable of streaming, which requires defining the column widths
/// beforehand. If a value is wider than the defined column width for
/// that value, a single space is still printed between the value and
/// the next. The last column does not need a width, and no padding is
/// printed.
pub struct TerminalTable<O: Write + IsTerminal> {
    pub opts: TerminalTableOpts,
    settings: TerminalTableSettings,
    thin_bar: String,
    thick_bar: String,
    out: BufWriter<O>,
}

impl<O: Write + IsTerminal> TerminalTable<O> {
    /// How many spaces to put between columns in human-readable
    /// format at minimum, even if a value is longer than anticipated.
    const MINIMAL_PADDING_LEN: usize = 1;

    /// The length of `widths` must be one less than that of `titles`
    /// (the last column does not need a width).  Appends a space to
    /// each title (or generally, formatted item), to make sure italic
    /// text is not clipped on terminals. That will be fine as you'll
    /// want your widths to be at least 1 longer than the text itself,
    /// anyway. `widths` must include the spacing between the
    /// columns--i.e. make it 2-3 larger than the max. expected width
    /// of the data.
    pub fn new(widths: &[usize], opts: TerminalTableOpts, out: O) -> Self {
        let max_width = widths.iter().max().copied().unwrap_or(0);
        let padding = " ".repeat(max_width);
        let is_terminal = out.is_terminal();

        let width = get_terminal_width(1);
        let bar_of = |c: &str| c.repeat(width) + "\n";
        let (thin_bar, thick_bar) = if *UNICODE_IS_FINE {
            (bar_of("─"), bar_of("═"))
        } else {
            (bar_of("-"), bar_of("="))
        };

        Self {
            settings: TerminalTableSettings {
                widths: widths.to_owned(),
                padding,
                is_terminal,
            },
            opts,
            out: BufWriter::new(out),
            thin_bar,
            thick_bar,
        }
    }
}

impl<O: Write + IsTerminal> OutputTable for TerminalTable<O> {
    type Output = O;

    fn num_columns(&self) -> usize {
        self.settings.widths.len() + 1
    }

    // Not making this an instance method so that we can give mut vs
    // non-mut parts independently
    fn write_row<'url, V: CellValue<'url>>(
        &mut self,
        row: Row<V>,
        line_style: Option<OutputStyle>,
    ) -> Result<()> {
        let (expected_num_columns, row_num_columns) = (self.num_columns(), row.logical_len());
        if expected_num_columns != row_num_columns {
            bail!(
                "the row contains {row_num_columns} instead of the expected \
                 {expected_num_columns} columns"
            )
        }

        let mut is_first = true;
        for (text, width_opt) in row.string_and_widths(&self.settings.widths) {
            if self.opts.tsv && !is_first {
                self.out.write_all("\t".as_bytes())?;
            }
            let mut text = text.to_string();
            let minimal_pading_len;
            if let Some(style) = line_style {
                // make sure italic text is not clipped on terminals
                text.push_str(" ");
                minimal_pading_len = Self::MINIMAL_PADDING_LEN.saturating_sub(1);
                let s = text.as_str().paint(style);
                let s = s.to_string();
                self.out.write_all(s.as_bytes())?;
            } else {
                minimal_pading_len = Self::MINIMAL_PADDING_LEN;
                self.out.write_all(text.as_bytes())?;
            }
            let text_len = text.len();

            if let Some(width) = width_opt {
                if !self.opts.tsv {
                    let missing_padding_len = width.saturating_sub(text_len);
                    let wanted_padding_len = missing_padding_len.max(minimal_pading_len);
                    let padding = &self.settings.padding[0..wanted_padding_len];
                    self.out.write_all(padding.as_bytes())?;
                }
            }

            is_first = false;
        }
        self.out.write_all(b"\n")?;
        Ok(())
    }

    fn write_title_row(
        &mut self,
        titles: &[OutputTableTitle],
        line_style: Option<OutputStyle>,
    ) -> Result<()> {
        let style = line_style.unwrap_or(OutputStyle {
            bold: true,
            italic: true,
            ..Default::default()
        });

        self.write_row(
            Row::<&str>::WithSpans(titles),
            if self.opts.want_color(self.settings.is_terminal) {
                Some(style)
            } else {
                None
            },
        )
    }

    fn write_thin_bar(&mut self) -> anyhow::Result<()> {
        Ok(self.out.write_all(self.thin_bar.as_bytes())?)
    }

    fn write_thick_bar(&mut self) -> anyhow::Result<()> {
        Ok(self.out.write_all(self.thick_bar.as_bytes())?)
    }

    fn print<'url, V: CellValue<'url>>(&mut self, value: V) -> anyhow::Result<()> {
        self.out.write_all(value.as_ref().as_bytes())?;
        Ok(())
    }

    fn finish(self) -> Result<O> {
        self.out
            .into_inner()
            .map_err(|e| anyhow!("flushing the buffer: {}", e.error()))
    }
}
