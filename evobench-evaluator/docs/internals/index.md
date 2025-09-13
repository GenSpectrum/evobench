# Source directory overview

## `bin` subdirectory

The source files representing program binaries.

These are the user-relevant programs:

* [`bin/evobench-evaluator.rs`](../../src/bin/evobench-evaluator.rs): produce human-readable outputs from benchmarking log files; does not know about where to place files (needs explicit paths), and doesn't know about running benchmarks
* [`bin/evobench-run.rs`](../../src/bin/evobench-run.rs): runs benchmarking jobs, i.e. produces benchmarking log files in a structured and automatic way (i.e. offers a service plus tools to change and query the service status); calls `evobench-evaluator` to turn them into human-readable outputs.

Other programs (not normally in use, feel free to ignore):

* [`bin/jobqueue.rs`](../../src/bin/jobqueue.rs): a general purpose program to work with queues (just an application of the `key_val_fs` module, perhaps generally useful?)
* [`bin/trying-git.rs`](../../src/bin/trying-git.rs): a program to play with git graphs, mostly to verify the workings of the `git` module.

## Other subdirectories

* [`serde/`](../../src/serde/mod.rs): custom types in config files and other places with user interaction via text
* [`key_val_fs/`](../../src/key_val_fs/mod.rs): a simple key-value database via files, and a queue implementation on top
* [`stats/`](../../src/stats/mod.rs): simple statistics, keeping track of the unit (ns, us, counts) via the type system
* [`tables/`](../../src/tables/mod.rs): tabular output for Excel, works with [`stats/`](../../src/stats/mod.rs) keeping track of the unit (ns, us, counts) via the type system
* [`evaluator/`](../../src/evaluator/mod.rs): the meat of the `evobench-evaluator` tool
* [`run/`](../../src/run/mod.rs): the meat of the `evobench-run` tool

(There are some more, utilities without group documentation:
[`date_and_time/`](../../src/date_and_time/mod.rs),
[`utillib/`](../../src/utillib/mod.rs),
[`io_utils/`](../../src/io_utils/mod.rs).)

## Tool internals documentation

* [evobench-evaluator](evaluator/index.md)

* [evobench-run](runner/index.md)
