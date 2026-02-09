use itertools::{EitherOrBoth, Itertools};
use std::borrow::Cow;
use std::fmt::Display;
use yansi::Style;
pub mod html;
pub mod terminal;

#[derive(Debug)]
pub struct OutputTableTitle<'s> {
    pub text: Cow<'s, str>,
    /// How many columns this should span across; should normally be
    /// `1`
    pub span: usize,
}

pub enum Row<'r, 's, V: Display> {
    WithSpans(&'r [OutputTableTitle<'s>]),
    PlainStrings(&'r [V]),
}

impl<'r, 's, V: Display> Row<'r, 's, V> {
    /// How many columns this Row covers (if it has entries that span
    /// multiple columns, all of those are added)
    fn logical_len(&self) -> usize {
        match self {
            Row::WithSpans(terminal_table_titles) => {
                let mut cols = 0;
                for OutputTableTitle { text: _, span } in *terminal_table_titles {
                    cols += span;
                }
                cols
            }
            Row::PlainStrings(items) => items.len(),
        }
    }

    /// Adds widths together for spanned columns. The width for the
    /// last column is None. -- This is only interesting for
    /// TerminalTable.
    fn string_and_widths(&self, widths: &[usize]) -> Vec<(Cow<'_, str>, Option<usize>)> {
        match self {
            Row::WithSpans(terminal_table_titles) => {
                let mut v: Vec<(Cow<str>, Option<usize>)> = Vec::new();
                let mut widths = widths.into_iter();
                for OutputTableTitle { text, span } in *terminal_table_titles {
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

pub trait OutputTable {
    type Output;

    /// Normally, use `write_title_row` or `write_data_row` instead!
    fn write_row<V: Display>(
        &mut self,
        row: Row<V>,
        line_style: Option<Style>,
    ) -> anyhow::Result<()>;

    fn write_title_row(
        &mut self,
        titles: &[OutputTableTitle],
        style: Option<Style>,
    ) -> anyhow::Result<()>;

    fn write_data_row<V: Display>(
        &mut self,
        data: &[V],
        line_style: Option<Style>,
    ) -> anyhow::Result<()> {
        self.write_row(Row::PlainStrings(data), line_style)
    }

    fn print(&mut self, s: &str) -> anyhow::Result<()>;

    fn finish(self) -> anyhow::Result<Self::Output>;
}
