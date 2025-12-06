use linfa::traits::*;
use linfa_clustering::Dbscan;
// use linfa_datasets::generate;
use ndarray::array;

fn main() {
    let observations = array![
        [0., 1.],
        [-10., 20.],
        [-1., 10.],
        [-1.1, 10.3],
        [-0.9, 10.2],
        [-10.3, 19.9],
    ];

    // Let's configure and run our DBSCAN algorithm
    // We use the builder pattern to specify the hyperparameters
    // `min_points` is the only mandatory parameter.
    // If you don't specify the others (e.g. `tolerance`)
    // default values will be used.
    let min_points = 2;
    let clusters = Dbscan::params(min_points)
        .tolerance(0.8)
        .transform(&observations)
        .unwrap();
    // Points are `None` if noise `Some(id)` if belonging to a cluster.
    dbg!(clusters);
}
