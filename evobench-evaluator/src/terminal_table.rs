//! Experimental attempt at a table printing abstraction that can both
//! print to a terminal in nice human-readable format (with spaces for
//! padding, and ANSI sequences for formatting), as well as in CSV
//! (with tabs) format.

//! Does not currently escape anything in the fields, just uses
//! `Display` and prints that directly. Thus is not safe if the type
//! can print tabs or newlines (or on the terminal even spaces could
//! make it ambiguous).

use std::{fmt::Display, io::Write};

use anyhow::{anyhow, bail, Result};
use itertools::Itertools;
use yansi::{Paint, Style};

/// Capable of streaming, which requires defining the column widths
/// beforehand. If a value is wider than the defined column width for
/// that value, a single space is still printed between the value and
/// the next. The last column does not need a width, and no padding is
/// printed.
pub struct TerminalTable {
    widths: Vec<usize>,
    titles: Vec<String>,
    padding: String,
    /// Whether to print as CSV (with tab as separator) and omit
    /// printing ANSI codes and padding.
    pub tsv_mode: bool,
}

impl TerminalTable {
    /// The length of `widths` must be one less than that of `titles`
    /// (the last column does not need a width).  Appends a space to
    /// each title, to make sure italic text is not clipped on
    /// terminals. That will be fine as you'll want your widths to be
    /// at least 1 longer than the text itself, anyway.
    pub fn new<S: Display>(widths: &[usize], titles: &[S], tsv_mode: bool) -> Self {
        let titles = titles.iter().map(|title| format!("{title} ")).collect();
        let max_width = widths.iter().max().copied().unwrap_or(0);
        let padding = " ".repeat(max_width);
        Self {
            widths: widths.to_owned(),
            titles,
            padding,
            tsv_mode,
        }
    }

    fn write_row<V: Display>(
        &self,
        row: &[V],
        line_style: Option<&Style>,
        out: &mut impl Write,
    ) -> Result<()> {
        let lens = (self.widths.len(), row.len());
        let (l1, l2) = lens;
        if l1
            != l2
                .checked_sub(1)
                .ok_or_else(|| anyhow!("need at least 1 column"))?
        {
            bail!("widths.len != data.len - 1: {lens:?}")
        }

        let mut is_first = true;
        for either_or_both in self.widths.iter().zip_longest(row) {
            if self.tsv_mode && !is_first {
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
                if !self.tsv_mode {
                    if *width > s_len {
                        let needed_padding = width - s_len;
                        let padding = &self.padding[0..needed_padding];
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

    pub fn write_title_row(&self, out: &mut impl Write) -> Result<()> {
        const STYLE: Style = Style::new().bold().italic();
        self.write_row(
            &self.titles,
            if self.tsv_mode { None } else { Some(&STYLE) },
            out,
        )
    }

    pub fn write_data_row<V: Display>(&self, data: &[V], out: &mut impl Write) -> Result<()> {
        self.write_row(data, None, out)
    }
}
