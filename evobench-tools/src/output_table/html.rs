use ahtml::{AVec, HtmlAllocator, Node, att, util::SoftPre};

use crate::output_table::OutputTable;

use super::{OutputTableTitle, Row};

pub struct HtmlTable<'allocator> {
    num_columns: usize,
    table_body: AVec<'allocator, Node>,
    // Don't need a separate allocator field since table_body carries it already.
    // html: &'allocator HtmlAllocator,
}

impl<'allocator> HtmlTable<'allocator> {
    pub fn new(num_columns: usize, html: &'allocator HtmlAllocator) -> Self {
        Self {
            num_columns,
            table_body: html.new_vec(),
        }
    }
}

impl<'allocator> OutputTable for HtmlTable<'allocator> {
    type Output = AVec<'allocator, Node>;

    fn num_columns(&self) -> usize {
        self.num_columns
    }

    fn write_row<V: AsRef<str>>(
        &mut self,
        row: Row<V>,
        line_style: Option<yansi::Style>,
    ) -> anyhow::Result<()> {
        let html = self.table_body.allocator();
        let mut cells = html.new_vec();
        match row {
            Row::WithSpans(items) => {
                for item in items {
                    let OutputTableTitle { text, span } = item;
                    let s: &str = text.as_ref();
                    cells.push(html.td([att("colspan", *span)], html.text(s)?)?)?;
                }
            }
            Row::PlainStrings(items) => {
                for item in items {
                    let s: &str = item.as_ref();
                    cells.push(html.td([], html.text(s)?)?)?;
                }
            }
        }
        self.table_body.push(html.tr([], cells)?)
    }

    fn write_title_row(
        &mut self,
        titles: &[OutputTableTitle],
        style: Option<yansi::Style>,
    ) -> anyhow::Result<()> {
        let html = self.table_body.allocator();
        let mut cells = html.new_vec();
        for item in titles {
            let OutputTableTitle { text, span } = item;
            let s: &str = text.as_ref();
            cells.push(html.th([att("colspan", *span)], html.text(s)?)?)?;
        }
        self.table_body.push(html.tr([], cells)?)
    }

    fn write_thin_bar(&mut self) -> anyhow::Result<()> {
        let html = self.table_body.allocator();
        self.table_body.push(html.tr(
            [],
            html.td(
                [att("colspan", self.num_columns())],
                html.hr([att("style", "width: 100%; border-style: dashed;")], [])?,
            )?,
        )?)
    }

    fn write_thick_bar(&mut self) -> anyhow::Result<()> {
        let html = self.table_body.allocator();
        self.table_body.push(html.tr(
            [],
            html.td(
                [att("colspan", self.num_columns())],
                html.hr([att("style", "width: 100%; ")], [])?,
            )?,
        )?)
    }

    fn print(&mut self, s: &str) -> anyhow::Result<()> {
        let html = self.table_body.allocator();
        let soft_pre = SoftPre {
            tabs_to_nbsp: Some(8),
            autolink: true,
            input_line_separator: "\n",
            trailing_br: false,
        };
        let text = soft_pre.format(s, html)?;
        self.table_body
            .push(html.tr([], html.td([att("colspan", self.num_columns())], text)?)?)
    }

    fn finish(self) -> anyhow::Result<Self::Output> {
        Ok(self.table_body)
    }
}
