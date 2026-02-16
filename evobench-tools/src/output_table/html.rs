use ahtml::{AVec, HtmlAllocator, Node, att, util::SoftPre};

use crate::{
    output_table::{CellValue, OutputStyle, OutputTable},
    utillib::html_util::anchor,
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
            font_size,
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
                4 => {
                    htmlcolor = {
                        // terminal color 4 is blue, but choose something
                        // else to differentiate from links
                        Some("#e46000")
                    }
                }
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

        if let Some(fs) = font_size {
            s.push_str("font-size: ");
            s.push_str(fs.as_ref());
            s.push_str("; ");
        }

        // trim whitespace at the end in place
        s.truncate(s.trim_end().len());

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

    fn write_row<'url, V: CellValue<'url>>(
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
                    let text_node = html.text(s)?;
                    let text = if let Some(style) = &htmlstyle {
                        html.span([att("style", style)], text_node)?
                    } else {
                        text_node
                    };
                    let content = if let Some(url) = item.perhaps_url() {
                        html.a([att("href", url)], text)?
                    } else {
                        text
                    };
                    let content = if let Some(name) = item.perhaps_anchor_name() {
                        anchor(name, content, html)?
                    } else {
                        content
                    };
                    cells.push(html.td([], content)?)?;
                }
            }
        }
        self.table_body.push(html.tr([], cells)?)
    }

    /// You might want to give an OutputStyle with a font_size that is
    /// larger (otherwise it is the default which is the same as the
    /// body text?).
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

    fn print<'url, V: CellValue<'url>>(&mut self, value: V) -> anyhow::Result<()> {
        let html = self.table_body.allocator();
        let soft_pre = SoftPre {
            tabs_to_nbsp: Some(8),
            autolink: true,
            input_line_separator: "\n",
            trailing_br: false,
        };
        let text = soft_pre.format(value.as_ref(), html)?;
        let contents = if let Some(link) = value.perhaps_url() {
            html.a([att("href", link)], text)?
        } else {
            text
        };
        self.table_body
            .push(html.tr([], html.td([att("colspan", self.num_columns())], contents)?)?)
    }

    fn finish(self) -> anyhow::Result<Self::Output> {
        Ok(self.table_body)
    }
}
