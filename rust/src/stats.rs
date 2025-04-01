#[derive(Debug)]
pub struct Stats {
    pub num_values: usize,
    pub sum: u128,
    pub average: u64, // rounded down
    pub tiles: Vec<(usize, u64)>,
}

impl Stats {
    pub fn from_values(mut vals: Vec<u64>) -> Self {
        let num_values = vals.len();
        let sum: u128 = vals.iter().map(|v| u128::from(*v)).sum();
        let average = sum / (num_values as u128);
        vals.sort();

        let flen = (num_values - 1) as f64;
        let mut tiles = Vec::new();
        for i in 0..=10 {
            let index = i as f64 / 10. * flen;
            let val = vals[index as usize];
            tiles.push((i, val));
        }

        // dbg!(vals.first());
        // dbg!(vals.last());

        Stats {
            num_values,
            sum,
            average: average.try_into().expect("always fit"),
            tiles,
        }
    }
}
