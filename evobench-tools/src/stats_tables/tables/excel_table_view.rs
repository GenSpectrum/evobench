use std::{borrow::Cow, path::Path};

use anyhow::{Context, Result, anyhow};
use cj_path_util::unix::polyfill::add_extension;
use rust_xlsxwriter::{Color, Format, FormatAlign, workbook::Workbook};

use super::table_view::{ColumnFormatting, Highlight, TableView, Unit};
use crate::io_utils::div::xrename;

/// How many characters to add to the automatic column width
/// calculation to try to avoid setting widths too small to accomodate
/// the strings in the cells.
const WIDTH_SAFETY_MARGIN_CHARS: f64 = 2.0;

pub fn excel_file_write<'t>(
    tables: impl IntoIterator<Item = &'t (dyn TableView + 't)>,
    file: &Path,
) -> Result<()> {
    let mut workbook = Workbook::new();

    for table in tables {
        let worksheet = workbook.add_worksheet();
        worksheet.set_name(table.table_name()).with_context(|| {
            anyhow!(
                "trying to use table name as worksheet name: {:?}",
                table.table_name()
            )
        })?;

        let _titles = table.table_view_header();
        let titles = (*_titles).as_ref();

        // Our own max width tracking, in characters
        let mut column_widths: Vec<usize> = titles.iter().map(|_| 1).collect();

        let mut rownum = 0;

        {
            // How many lines do our labels take max?
            let mut num_lines = 1;
            for (i, (label, unit, _column_formatting)) in titles.iter().enumerate() {
                // write cell
                {
                    let colnum =
                        u16::try_from(i).with_context(|| anyhow!("too many columns for excel"))?;
                    let perhaps_unit: Cow<str> = match unit {
                        Unit::None => "".into(),
                        Unit::DimensionLess => "".into(),
                        Unit::Count => "\n(count)".into(),
                        Unit::ViewType(unit) => format!("\n({unit})").into(),
                    };
                    let val = format!("{label}{perhaps_unit}");
                    {
                        let max_width = val
                            .split('\n')
                            .map(|s| s.chars().count())
                            .max()
                            .unwrap_or(0);
                        column_widths[i] = column_widths[i].max(max_width);
                    }
                    let format = Format::new().set_bold();
                    worksheet
                        .write_with_format(rownum, colnum, &val, &format)
                        .with_context(|| anyhow!("write title value {val:?}"))?;
                }

                // update num_lines
                {
                    let label_linebreaks = label.chars().filter(|c| *c == '\n').count();
                    let unit_lines = match unit {
                        Unit::None => 0,
                        Unit::DimensionLess => 0,
                        Unit::Count => 1,
                        Unit::ViewType(_) => 1,
                    };
                    num_lines = num_lines.max(label_linebreaks + 1 + unit_lines);
                }
            }

            let height = (num_lines * 15) as f64;
            worksheet
                .set_row_height(rownum, height)
                .with_context(|| anyhow!("setting height of row {rownum} to height {height}"))?;
        }

        for row in table.table_view_body() {
            rownum += 1;
            for (i, (val, highlight)) in row.iter().enumerate() {
                let column_formatting: ColumnFormatting = titles[i].2;
                let colnum =
                    u16::try_from(i).with_context(|| anyhow!("too many columns for excel"))?;

                let mut format = Format::new();
                match column_formatting {
                    ColumnFormatting::Spacer => (),
                    ColumnFormatting::Number => {
                        format = format.set_align(FormatAlign::Right);
                    }
                    ColumnFormatting::String { width_chars: _ } => (),
                }
                match highlight {
                    Highlight::Spacer => (),
                    Highlight::Neutral => (),
                    Highlight::Red => {
                        format = format.set_font_color(Color::Red);
                    }
                    Highlight::Green => {
                        format = format.set_background_color(Color::Green);
                    }
                }

                {
                    let max_width = val
                        .split('\n')
                        .map(|s| s.chars().count())
                        .max()
                        .unwrap_or(0);
                    column_widths[i] = column_widths[i].max(max_width);
                }

                worksheet
                    .write_with_format(rownum, colnum, val.as_ref(), &format)
                    .with_context(|| anyhow!("write value {val:?}"))?;
            }
        }

        // Set column widths
        {
            // Note: newer versions of rust_xlsxwriter have
            // `autofit_to_max_width(&mut self, max_autofit_width: u16)`;
            // with this version, instead first autofit the whole sheet,
            // then fix other columns.
            //worksheet.autofit();
            // Actually that works very badly for our number (for
            // LibreOffice, anyway). So use our own character
            // counting.
            for (i, num_chars) in column_widths.iter().enumerate() {
                let colnum =
                    u16::try_from(i).with_context(|| anyhow!("too many columns for excel"))?;
                let width = *num_chars as f64 + WIDTH_SAFETY_MARGIN_CHARS;
                worksheet.set_column_width(colnum, width).with_context(|| {
                    anyhow!("setting column width on column {colnum} to {width}")
                })?;
            }

            for (i, (_label, _unit, column_formatting)) in titles.as_ref().iter().enumerate() {
                let colnum =
                    u16::try_from(i).with_context(|| anyhow!("too many columns for excel"))?;

                match column_formatting {
                    ColumnFormatting::Spacer => {
                        worksheet
                            .set_column_width(colnum, 3.0)
                            .with_context(|| anyhow!("setting column width on column {colnum}"))?;
                    }
                    ColumnFormatting::Number => {
                        // Alignment already done while writing the cells.

                        // Autofit already done above. Newer versions of
                        // rust_xlsxwriter instead:
                        // worksheet.autofit_to_max_width(max_autofit_width:
                        // u16).
                    }
                    ColumnFormatting::String { width_chars } => {
                        if let Some(width_chars) = width_chars {
                            worksheet
                                .set_column_width(colnum, *width_chars)
                                .with_context(|| {
                                    anyhow!("setting column width on column {colnum}")
                                })?;
                        }
                    }
                }
            }
        }
    }

    let file_tmp =
        add_extension(file, "tmp").ok_or_else(|| anyhow!("path misses a filename: {file:?}"))?;
    workbook
        .save(&file_tmp)
        .with_context(|| anyhow!("saving to file {file_tmp:?}"))?;
    xrename(&file_tmp, &file)?;

    Ok(())
}
