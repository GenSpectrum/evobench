use std::fmt::Display;
use std::marker::PhantomData;

#[derive(Debug)]
pub struct Stats<ViewType: From<u64>> {
    view_type: PhantomData<fn() -> ViewType>,
    pub num_values: usize,
    pub sum: u128,
    pub average: u64, // rounded down
    /// Percentiles or in whatever number of sections you asked:
    /// sample count is the index, the sample value there is the value
    /// in the vector.
    pub tiles: Vec<u64>,
}

impl<ViewType: From<u64>> Stats<ViewType> {
    /// `tiles_count` is how many 'tiles' to build, for percentiles
    /// give the number 101.
    pub fn from_values(mut vals: Vec<u64>, tiles_count: usize) -> Self {
        let num_values = vals.len();
        let sum: u128 = vals.iter().map(|v| u128::from(*v)).sum();
        let average = sum / (num_values as u128);
        vals.sort();

        let flen = (num_values - 1) as f64;
        let mut tiles = Vec::new();
        let tiles_max = tiles_count as f64;
        for i in 0..=tiles_count {
            let index = i as f64 / tiles_max * flen;
            let val = vals[index as usize];
            tiles.push(val);
        }

        // dbg!(vals.first());
        // dbg!(vals.last());

        Stats {
            view_type: PhantomData::default(),
            num_values,
            sum,
            average: average.try_into().expect("always fit"),
            tiles,
        }
    }

    /// Uses the values from `tiles`; panics if you gave an even
    /// tiles_count (must be odd so the middle is present)
    pub fn median(&self) -> u64 {
        assert!(0 == self.tiles.len() % 2);
        self.tiles[self.tiles.len() / 2]
    }
}

impl<ViewType: From<u64> + Display> Display for Stats<ViewType> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            view_type: _,
            num_values,
            sum,
            average,
            tiles: _,
        } = self;
        writeln!(
            f,
            " {num_values} values \t sum {} \t average {} \t median {}",
            ViewType::from(u64::try_from(*sum).expect("sum is larger than u64: {sum}")),
            ViewType::from(*average),
            ViewType::from(self.median())
        )
    }
}
