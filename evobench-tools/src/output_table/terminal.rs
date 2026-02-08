//! Experimental attempt at a table printing abstraction that can both
//! print to a terminal in nice human-readable format (with spaces for
//! padding, and ANSI sequences for formatting), as well as in CSV
//! (with tabs) format.

//! Does not currently escape anything in the fields, just uses
//! `Display` and prints that directly. Thus is not safe if the type
//! can print tabs or newlines (or on the terminal even spaces could
//! make it ambiguous).

use std::{
    borrow::Cow,
    fmt::Display,
    io::{BufWriter, IsTerminal, Write},
};

use anyhow::{Result, anyhow, bail};
use itertools::{EitherOrBoth, Itertools};
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

#[derive(Debug)]
pub struct TerminalTableTitle<'s> {
    pub text: Cow<'s, str>,
    /// How many columns this should span across; should normally be
    /// `1`
    pub span: usize,
}

enum Row<'r, 's, V: Display> {
    WithSpans(&'r [TerminalTableTitle<'s>]),
    PlainStrings(&'r [V]),
}

impl<'r, 's, V: Display> Row<'r, 's, V> {
    /// How many columns this Row covers (if it has entries that span
    /// multiple columns, all of those are added)
    fn logical_len(&self) -> usize {
        match self {
            Row::WithSpans(terminal_table_titles) => {
                let mut cols = 0;
                for TerminalTableTitle { text: _, span } in *terminal_table_titles {
                    cols += span;
                }
                cols
            }
            Row::PlainStrings(items) => items.len(),
        }
    }

    /// Adds widths together for spanned columns. The width for the
    /// last column is None.
    fn string_and_widths(&self, widths: &[usize]) -> Vec<(Cow<'_, str>, Option<usize>)> {
        match self {
            Row::WithSpans(terminal_table_titles) => {
                let mut v: Vec<(Cow<str>, Option<usize>)> = Vec::new();
                let mut widths = widths.into_iter();
                for TerminalTableTitle { text, span } in *terminal_table_titles {
                    match *span {
                        0 => (),
                        n => {
                            let width = (|| {
                                let mut tot_width = 0;
                                for _ in 0..n {
                                    if let Some(width) = widths.next() {
                                        tot_width += width;
                                    } else {
                                        return None;
                                    }
                                }
                                Some(tot_width)
                            })();
                            v.push((text.as_ref().into(), width));
                        }
                    }
                }
                v
            }
            Row::PlainStrings(items) => {
                let mut v: Vec<(Cow<str>, Option<usize>)> = Vec::new();
                for either_or_both in items.iter().zip_longest(widths) {
                    match either_or_both {
                        EitherOrBoth::Both(val, width) => {
                            v.push((val.to_string().into(), Some(*width)))
                        }
                        EitherOrBoth::Left(val) => v.push((val.to_string().into(), None)),
                        EitherOrBoth::Right(_) => {
                            unreachable!("given row len has been checked against widths len")
                        }
                    }
                }
                v
            }
        }
    }
}

struct TerminalTableSettings<'v, 's> {
    widths: Vec<usize>,
    titles: &'v [TerminalTableTitle<'s>],
    padding: String,
    is_terminal: bool,
}

/// Capable of streaming, which requires defining the column widths
/// beforehand. If a value is wider than the defined column width for
/// that value, a single space is still printed between the value and
/// the next. The last column does not need a width, and no padding is
/// printed.
pub struct TerminalTable<'v, 's, O: Write + IsTerminal> {
    pub opts: TerminalTableOpts,
    settings: TerminalTableSettings<'v, 's>,
    out: BufWriter<O>,
}

impl<'v, 's, O: Write + IsTerminal> TerminalTable<'v, 's, O> {
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
    pub fn start(
        widths: &[usize],
        titles: &'v [TerminalTableTitle<'s>],
        style: Option<Style>,
        opts: TerminalTableOpts,
        out: O,
    ) -> Result<Self> {
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
        if let Some(style) = style {
            slf.write_title_row_with_style(style)?;
        } else {
            slf.write_title_row()?;
        }
        Ok(slf)
    }

    // Not making this an instance method so that we can give mut vs
    // non-mut parts independently
    fn write_row<V: Display>(
        opts: &TerminalTableOpts,
        settings: &TerminalTableSettings,
        out: &mut BufWriter<O>,
        row: Row<V>,
        line_style: Option<Style>,
    ) -> Result<()> {
        let lens = (settings.widths.len(), row.logical_len());
        let (l1, l2) = lens;
        if l1
            != l2
                .checked_sub(1)
                .ok_or_else(|| anyhow!("need at least 1 column"))?
        {
            bail!("widths.len != data.len - 1: {lens:?}")
        }

        let mut is_first = true;
        for (text, width_opt) in row.string_and_widths(&settings.widths) {
            if opts.tsv && !is_first {
                out.write_all("\t".as_bytes())?;
            }
            let mut text = text.to_string();
            let minimal_pading_len;
            if let Some(style) = line_style {
                // make sure italic text is not clipped on terminals
                text.push_str(" ");
                minimal_pading_len = Self::MINIMAL_PADDING_LEN.saturating_sub(1);
                let s = text.as_str().paint(style);
                let s = s.to_string();
                out.write_all(s.as_bytes())?;
            } else {
                minimal_pading_len = Self::MINIMAL_PADDING_LEN;
                out.write_all(text.as_bytes())?;
            }
            let text_len = text.len();

            if let Some(width) = width_opt {
                if !opts.tsv {
                    let missing_padding_len = width.saturating_sub(text_len);
                    let wanted_padding_len = missing_padding_len.max(minimal_pading_len);
                    let padding = &settings.padding[0..wanted_padding_len];
                    out.write_all(padding.as_bytes())?;
                }
            }

            is_first = false;
        }
        out.write_all(b"\n")?;
        Ok(())
    }

    pub fn write_title_row_with_style(&mut self, style: Style) -> Result<()> {
        Self::write_row(
            &self.opts,
            &self.settings,
            &mut self.out,
            Row::<&str>::WithSpans(self.settings.titles),
            if self.opts.want_color(self.settings.is_terminal) {
                Some(style)
            } else {
                None
            },
        )
    }

    pub fn write_title_row(&mut self) -> Result<()> {
        const STYLE: Style = Style::new().bold().italic();
        self.write_title_row_with_style(STYLE)
    }

    pub fn write_data_row<V: Display>(
        &mut self,
        data: &[V],
        line_style: Option<Style>,
    ) -> Result<()> {
        Self::write_row(
            &self.opts,
            &self.settings,
            &mut self.out,
            Row::PlainStrings(data),
            line_style,
        )
    }

    pub fn print(&mut self, s: &str) -> Result<()> {
        self.out.write_all(s.as_bytes())?;
        Ok(())
    }

    pub fn finish(self) -> Result<O> {
        self.out
            .into_inner()
            .map_err(|e| anyhow!("flushing the buffer: {}", e.error()))
    }
}
