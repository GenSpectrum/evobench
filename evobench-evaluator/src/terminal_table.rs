//! Experimental attempt at a table printing abstraction that can both
//! print to a terminal in nice human-readable format (with spaces for
//! padding, and ANSI sequences for formatting), as well as in CSV
//! (with tabs) format.

//! Does not currently escape anything in the fields, just uses
//! `Display` and prints that directly. Thus is not safe if the type
//! can print tabs or newlines (or on the terminal even spaces could
//! make it ambiguous).

use std::{
    fmt::Display,
    io::{BufWriter, IsTerminal, Write},
};

use anyhow::{anyhow, bail, Result};
use itertools::Itertools;
use strum_macros::EnumString;
use yansi::{Paint, Style};

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
    /// Whether to show the table as CSV (with '\t' as separator)
    /// instead of human-readable
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
    titles: Vec<String>,
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
    out: BufWriter<O>,
}

impl<O: Write + IsTerminal> TerminalTable<O> {
    /// The length of `widths` must be one less than that of `titles`
    /// (the last column does not need a width).  Appends a space to
    /// each title, to make sure italic text is not clipped on
    /// terminals. That will be fine as you'll want your widths to be
    /// at least 1 longer than the text itself, anyway.
    pub fn start<S: Display>(
        widths: &[usize],
        titles: &[S],
        opts: TerminalTableOpts,
        out: O,
    ) -> Result<Self> {
        let titles = titles.iter().map(|title| format!("{title} ")).collect();
        let max_width = widths.iter().max().copied().unwrap_or(0);
        let padding = " ".repeat(max_width);
        let is_terminal = out.is_terminal();
        let mut slf = Self {
            settings: TerminalTableSettings {
                widths: widths.to_owned(),
                titles,
                padding,
                is_terminal,
            },
            opts,
            out: BufWriter::new(out),
        };
        slf.write_title_row()?;
        Ok(slf)
    }

    // Not making this an instance method so that we can give mut vs
    // non-mut parts independently
    fn write_row<V: Display>(
        opts: &TerminalTableOpts,
        settings: &TerminalTableSettings,
        out: &mut BufWriter<O>,
        row: &[V],
        line_style: Option<&Style>,
    ) -> Result<()> {
        let lens = (settings.widths.len(), row.len());
        let (l1, l2) = lens;
        if l1
            != l2
                .checked_sub(1)
                .ok_or_else(|| anyhow!("need at least 1 column"))?
        {
            bail!("widths.len != data.len - 1: {lens:?}")
        }

        let mut is_first = true;
        for either_or_both in settings.widths.iter().zip_longest(row) {
            if opts.tsv && !is_first {
                out.write_all("\t".as_bytes())?;
            }

            let val = either_or_both
                .as_ref()
                .right()
                .expect("value there because row len checked above");
            let s = val.to_string();
            let s_len = s.len();
            {
                let s: String = if let Some(style) = line_style {
                    let s = s.paint(*style);
                    s.to_string()
                } else {
                    s
                };
                out.write_all(s.as_bytes())?;
            }

            if let Some(width) = either_or_both.left() {
                if !opts.tsv {
                    if *width > s_len {
                        let needed_padding = width - s_len;
                        let padding = &settings.padding[0..needed_padding];
                        out.write_all(padding.as_bytes())?;
                    } else {
                        // write out at least 1 space anyway
                        out.write_all(b" ")?;
                    }
                }
            }

            is_first = false;
        }
        out.write_all(&[b'\n'])?;
        Ok(())
    }

    pub fn write_title_row(&mut self) -> Result<()> {
        const STYLE: Style = Style::new().bold().italic();
        Self::write_row(
            &self.opts,
            &self.settings,
            &mut self.out,
            &self.settings.titles,
            if self.opts.want_color(self.settings.is_terminal) {
                Some(&STYLE)
            } else {
                None
            },
        )
    }

    pub fn write_data_row<V: Display>(&mut self, data: &[V]) -> Result<()> {
        Self::write_row(&self.opts, &self.settings, &mut self.out, data, None)
    }

    pub fn finish(self) -> Result<O> {
        self.out
            .into_inner()
            .map_err(|e| anyhow!("flushing the buffer: {}", e.error()))
    }
}
