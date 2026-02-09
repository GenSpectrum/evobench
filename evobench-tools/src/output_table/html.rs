use ahtml::{AVec, HtmlAllocator, Node, att, util::SoftPre};

use crate::{
    output_table::{CellValue, OutputStyle, OutputTable},
    warn,
};

use super::{OutputTableTitle, Row};

impl OutputStyle {
    fn to_css_style(self) -> String {
        let OutputStyle {
            faded,
            bold,
            italic,
            color,
        } = self;

        let mut s = String::new();

        if italic {
            s.push_str("font-style: italic; ");
        }
        if bold {
            s.push_str("font-weight: bold; ");
        }

        let mut htmlcolor: Option<&str> = None;
        if let Some(col) = color {
            match col {
                4 => htmlcolor = Some("blue"),
                _ => {
                    warn!("ignoring unknown color code {col}");
                }
            }
        }
        if faded {
            if htmlcolor.is_some() {
                warn!(
                    "both 'faded' and 'color' were given, ignoring 'faded' (should it shade color?)"
                );
            } else {
                htmlcolor = Some("gray");
            }
        }
        if let Some(htmlcol) = htmlcolor {
            s.push_str("color: ");
            s.push_str(htmlcol);
            s.push_str("; ");
        }

        s
    }
}

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

    fn write_row<V: CellValue>(
        &mut self,
        row: Row<V>,
        line_style: Option<OutputStyle>,
    ) -> anyhow::Result<()> {
        let htmlstyle = line_style.map(OutputStyle::to_css_style);
        let html = self.table_body.allocator();
        let mut cells = html.new_vec();
        match row {
            Row::WithSpans(items) => {
                for item in items {
                    let OutputTableTitle { text, span } = item;
                    let s: &str = text.as_ref();
                    let text_node = html.text(s)?;
                    let text = if let Some(style) = &htmlstyle {
                        html.span([att("style", style)], text_node)?
                    } else {
                        text_node
                    };
                    cells.push(html.td([att("colspan", *span)], text)?)?;
                }
            }
            Row::PlainStrings(items) => {
                for item in items {
                    let s: &str = item.as_ref();
                    let text = html.text(s)?;
                    let content = if let Some(url) = item.perhaps_url() {
                        html.a([att("href", url)], text)?
                    } else {
                        text
                    };
                    cells.push(html.td([], content)?)?;
                }
            }
        }
        self.table_body.push(html.tr([], cells)?)
    }

    fn write_title_row(
        &mut self,
        titles: &[OutputTableTitle],
        line_style: Option<OutputStyle>,
    ) -> anyhow::Result<()> {
        let htmlstyle = line_style.map(OutputStyle::to_css_style);
        let html = self.table_body.allocator();
        let mut cells = html.new_vec();
        for item in titles {
            let OutputTableTitle { text, span } = item;
            let s: &str = text.as_ref();
            let text_node = html.text(s)?;
            let text = if let Some(style) = &htmlstyle {
                html.span([att("style", style)], text_node)?
            } else {
                text_node
            };
            cells.push(html.th([att("colspan", *span)], text)?)?;
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
