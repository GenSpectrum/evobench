use itertools::{EitherOrBoth, Itertools};
use std::borrow::Cow;

pub mod html;
pub mod terminal;

#[derive(Debug)]
pub struct OutputTableTitle<'s> {
    pub text: Cow<'s, str>,
    /// How many columns this should span across; should normally be
    /// `1`
    pub span: usize,
}

pub trait CellValue: AsRef<str> {
    /// If appropriate for the type and instance, return a URL value
    fn perhaps_url(&self) -> Option<String>;
}

impl CellValue for &str {
    fn perhaps_url(&self) -> Option<String> {
        None
    }
}
impl CellValue for String {
    fn perhaps_url(&self) -> Option<String> {
        None
    }
}
impl<'t> CellValue for Cow<'t, str> {
    fn perhaps_url(&self) -> Option<String> {
        None
    }
}
// Hmm huh.
impl<'t> CellValue for &dyn CellValue {
    fn perhaps_url(&self) -> Option<String> {
        (*self).perhaps_url()
    }
}

/// Either something that can have spans; or something that can have
/// URLs. Assumes that never want to have both.
pub enum Row<'r, 's, V: CellValue> {
    WithSpans(&'r [OutputTableTitle<'s>]),
    PlainStrings(&'r [V]),
}

impl<'r, 's, V: CellValue> Row<'r, 's, V> {
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
                            v.push((val.as_ref().into(), Some(*width)))
                        }
                        EitherOrBoth::Left(val) => v.push((val.as_ref().into(), None)),
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

/// Abstract styling that works for both terminal and HTML
/// output. `color`, if given, is a ANSI 256-color terminal color.
#[derive(Debug, Clone, Copy, Default)]
pub struct OutputStyle {
    pub faded: bool,
    pub bold: bool,
    pub italic: bool,
    pub color: Option<u8>,
}

pub trait OutputTable {
    type Output;

    /// How many columns this table has (each row has the same number
    /// of columns, although cells can span multiple columns)
    fn num_columns(&self) -> usize;

    /// Normally, use `write_title_row` or `write_data_row` instead!
    fn write_row<V: CellValue>(
        &mut self,
        row: Row<V>,
        line_style: Option<OutputStyle>,
    ) -> anyhow::Result<()>;

    fn write_title_row(
        &mut self,
        titles: &[OutputTableTitle],
        line_style: Option<OutputStyle>,
    ) -> anyhow::Result<()>;

    fn write_data_row<V: CellValue>(
        &mut self,
        data: &[V],
        line_style: Option<OutputStyle>,
    ) -> anyhow::Result<()> {
        self.write_row(Row::PlainStrings(data), line_style)
    }

    fn write_thin_bar(&mut self) -> anyhow::Result<()>;

    fn write_thick_bar(&mut self) -> anyhow::Result<()>;

    fn print(&mut self, s: &str) -> anyhow::Result<()>;

    fn finish(self) -> anyhow::Result<Self::Output>;
}
