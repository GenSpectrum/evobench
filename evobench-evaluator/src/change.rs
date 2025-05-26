use std::{borrow::Cow, marker::PhantomData};

use crate::table_view::{ColumnFormatting, Highlight, TableViewRow, Unit};

pub trait IsBetter {
    const FORMATTING_FOR_LARGER: Highlight;
    const FORMATTING_FOR_SMALLER: Highlight;
}

pub struct LargerIsBetter;
impl IsBetter for LargerIsBetter {
    const FORMATTING_FOR_LARGER: Highlight = Highlight::Green;

    const FORMATTING_FOR_SMALLER: Highlight = Highlight::Red;
}

pub struct SmallerIsBetter;
impl IsBetter for SmallerIsBetter {
    const FORMATTING_FOR_LARGER: Highlight = Highlight::Red;

    const FORMATTING_FOR_SMALLER: Highlight = Highlight::Green;
}

#[derive(Debug)]
pub struct Change<Better: IsBetter> {
    better: PhantomData<Better>,
    pub from: u64,
    pub to: u64,
}

impl<Better: IsBetter> Change<Better> {
    // XX take two `ViewType`s instead to ensure the values are
    // compatible? "But" already have u64 from `Stat`, "that's more
    // efficient".
    pub fn new(from: u64, to: u64) -> Self {
        Self {
            better: Default::default(),
            from,
            to,
        }
    }
}

impl<Better: IsBetter> TableViewRow<()> for Change<Better> {
    fn table_view_header(_: ()) -> Box<dyn AsRef<[(Cow<'static, str>, Unit, ColumnFormatting)]>> {
        const HEADER: &[(Cow<'static, str>, Unit, ColumnFormatting)] = &[(
            Cow::Borrowed("change"),
            Unit::DimensionLess,
            ColumnFormatting::Number,
        )];
        Box::new(HEADER)
    }
    fn table_view_row(&self, out: &mut Vec<(Cow<str>, Highlight)>) {
        let Change {
            better: _,
            from,
            to,
        } = self;
        let relative = *to as f64 / *from as f64;
        let formatting = if relative > 1.1 {
            Better::FORMATTING_FOR_LARGER
        } else if relative < 0.9 {
            Better::FORMATTING_FOR_SMALLER
        } else {
            Highlight::Neutral
        };
        out.push((format!("{relative:.3}").into(), formatting));
    }
}
