# How `evobench-evaluator` works internally

## Statistics levels

1. The benchmark log file resulting from a benchmarking run is
   processed to a statistics called "single" (for "single run"). Probe
   timings are collected into a tree so that for each dynamic location
   of the probe within the runtime call graph a path (like a
   backtrace, but only containing probe names, not function names) can
   be derived. For each such location within each thread (optionally),
   but also across threads, but also for the probes irrespective of
   location in the call graph, timings are collected and represented
   with statistical values (count, sum, average, standard deviation,
   median, percentiles) as a row in the Excel file; for flamegraphs,
   only the path based representation is used.

2. If there is an interest in detecting performance deviations,
   multiple benchmarking runs (e.g. 5 or 10) should be executed for a
   single combination of commit id of the target project and
   benchmarking invocation parameters (directory within target
   project, command and arguments, and environment variables if any),
   so that statistical significance for a deviation can be
   calculated. `evobench-evaluator` is run with the `summary`
   subcommand to calculate this second statistical level: the
   statistics for a particular result of each benchmarking run
   (example: take the *median* values of each probe-location of each
   run, calculate the count, sum, average, standard deviation, median
   and percentiles for *those*).
   
3. Then, given benchmarking logs from multiple commit ids (with
   multiple runs each), a trend or graph can be derived or performance
   deviation be calculate and reported. This third level is not
   implemented yet (but much has been prepared for it already).

## Types

### `options.rs`

The evaluator translates to Excel or flamegraph files (and in the
FUTURE: caches, graphs, perhaps reports).

It can translate to both of those output types in the same run: the
paths are specified in `OutputOpts`. They are given as options on the
same level (via `#[clap(flatten)]` from the
[clap](https://crates.io/crates/clap) command line parser crate) as
the parameters for the evaluation, which are in `EvaluationOpts`.

`OutputOpts` is checked and converted to `CheckedOutputOptions` before
use, which wraps a `OutputVariants`, which is a parameterized type
that holds Excel and flamegraph variants of data through the pipeline.

#### StatsField

When summarizing data (i.e. level 2 or 3 as described in [Statistics
levels](#Statistics levels) above), but also when generating
flamegraphs, a decision has to be taken about which statistical number
to build the higher level statistical evaluation over. The selection
of the field is represented by the `stats::StatsField<TILE_COUNT>`
enum type; the type parameter is an integer for how many tiles are
used in the statistics, currently the `evobench-evaluator` uses 101
everywhere (percentiles, 0..100 inclusive). To be used as command line
option, it implements `FromStr`, i.e. can be created from a string
(like "average", "stdev", "10").

This field is used in the types `evaluator::options::FlameFieldOpt`
(choice of field for the flamegraph output),
`evaluator::options::FieldSelectorDimension3Opt` (choice of field for
the level 2 statistics (summary)), and
`evaluator::options::FieldSelectorDimension4Opt` (choice of field for
the unfinished level 3 statistics). The point of these wrapper types
is to hold both help text and default value for `clap` as much as to
disambiguate the option usage in the code.

## Processing chain

### 1. Parsing and tree building

This part of the processing is done by the code in [evaluator/data/](../../../src/evaluator/data/mod.rs).

1. Parsing:

    The benchmarking log files are currently in an NLJSON based
    format, with version and context information at the beginning,
    optionally zstd compressed. The log lines are parsed into a
    vector of
    [`LogMessage`](../../../src/evaluator/data/log_message.rs), which
    contain [`Timing`](../../../src/evaluator/data/log_message.rs)
    records for probes, held by a
    [`LogData`](../../../src/evaluator/data/log_data.rs) instance.
    
    Note that `Timing` records contain just a single absolute data
    point (but for multiple different kinds of values, e.g. real time,
    cpu time etc.); it is by later pairing up the `Timing` records for
    the start (logging from object constructor) and end (logging from
    object destructor) of the same scope (identified by scope name,
    which must be unique!) and taking the difference that the cost
    becomes known. This design (calculating the difference during
    evaluation, not recording) was chosen to try to keep the cost of
    logging lower, but potentially the absolute timings could allow
    for additional evaluations (e.g. end of scope to end of parent
    scope) or event correlations, too (not currently done).

2. Tree building:

    Then `LogMessage` entries for probes (more precisely, references
    to their `Timing` parts, with the timings for the scope start and
    end for each probe paired up) are collected into a
    [`LogDataTree`](../../../src/evaluator/data/log_data_tree.rs). Both
    the LogData and derived LogDataTree are bundled in a
    [`LogDataAndTree`](../../../src/evaluator/data/log_data_and_tree.rs)
    instance.

    [evaluator/data/log_data_tree.rs (`path_string()` on `Span`)](../../../src/evaluator/data/log_data_tree.rs)
    also contains the code to turn a location in the tree into a path
    ("probe-span backtrace").

### 2. Path index, calculating statistics, collection into tables

#### Path index

The `LogDataAndTree` structure from the previous step contains all the
original, individual `Timing` records, two per each logging probe
encounter (the `EVOBENCH_SCOPE_EVERY` probes only log once for every n
encounters): one for the start and one for when the scope ends and the
destructor runs. The tree just holds them together according to the
dynamic context (thread, then call context). This detail data now
needs to be condensed down as descriptive statistics.

There are multiple ways how the tree could be condensed down: 

- One might wish to know the total cost of a particular scope,
  irrespective of its dynamic context (i.e. regardless where it was
  called from).
  
- Or one might wish to know the total cost of a particular scope *in a
  particular calling context*. In that case, 
  
    - one might also care about which thread that context (call path)
      was executed on,
    - or one might just want to know the total cost of the same call
      path across all threads.

The tree in `LogDataAndTree` has the most precise location
information. Some of that location information needs to be ignored for
collecting the `Timing` entries for the statistics, depending on the
interest as listed above.

In each case, a human-readable description of what the statistics was
calculated about (the location or overlaid locations in the tree) is
needed. A path string with separators and a few more features is
chosen for this; the `evaluator::data::Span::path_string` method
produces those strings. (For performance reasons, it generates these
strings into a mutable reference into a string, and for that reason
there is no custom type definition for those path strings.) This
method takes a `PathStringOptions` value to specify the details how
the path should be generated, e.g. whether the thread should be
mentioned or not, etc. The same location (represented by a
`evaluator::data::Span`) could produce such different path strings as:

1. With the specific thread (threads are numbered in order of new
   thread ids in timings occurring in the log, starting from 0):

        N:thread00 > main|main > sum_of_fibs|all > sum_of_fibs n=22 > sum_of_fibs|body > main|fib > fib|fib

2. Union across all threads:

        A:thread > main|main > sum_of_fibs|all > sum_of_fibs n=22 > sum_of_fibs|body > main|fib > fib|fib

3. The same path in reverse order:

        AR:fib|fib < main|fib < sum_of_fibs|body < sum_of_fibs n=22 < sum_of_fibs|all < main|main < thread

4. Or ignoring location altogether (only showing the probe name, not the location):

        fib|fib

Path 1 will represent the fewest data points since it is the most
specific, 2 and 3 (representing the same data points) represent
possibly more points since those paths potentially cover multiple
threads, 4 represents the most data points.

So, to collect the data points, for each point (again, represented by
a `evaluator::data::Span`) the path is calculated according to a
chosen `PathStringOptions` value, and then the path is keyed into a
hash map, and a reference to that `Span` is added to a vector held in
that map. Afterwards, for each entry in the map, the statistics over
its vector can be calculated. This map is wrapped in the
`evaluator::IndexByCallPath` type, and the indexing happens in the
`evaluator::IndexByCallPath::from_logdataindex` method.

Some of the parameters for generating the paths can be chosen via
command line arguments to `evobench-evaluator` (for the `single` or
`summary` subcommands). But for Excel output, multiple runs are done
with different `PathStringOptions` options to fill the
`IndexByCallPath` with entries for different usecases at once: the
resulting Excel sheets are "multi use" in this regard; the path
formats are chosen so that the generated paths are not ambiguous for
those cases.

#### Calculating statistics

So each path in `evaluator::IndexByCallPath` maps to the vector of
spans of timings for that path. Statistics are calculated for each of
those vectors, separately for each of the fields in the timings that
the user (explicitly or implicitly) is interested in. (We have 2
dimensions of statistical output here: the paths are one dimension,
the field the second dimension (although that one has a statically
fixed selection of values--"real time", "cpu time" etc.).)

Remember, the `Timing` records contain all the kinds of timings that
are collected: real time, cpu time, system time, multiple kinds of
context switches, and more. Some are not currently generated on macOS,
thus the currently extracted values are currently just real, cpu,
system times and a sum of all kinds of context switches.

For each of those timing kinds, a separate statistics is
calculated. For Excel output, the statistics for all timing kinds are
integrated into the same file as separate worksheets. For flamegraphs,
a separate SVG file is generated for each kind, adding the timing kind
name to the file name (like `single-real time.svg`, `single-cpu
time.svg`, etc.).

The `evaluator::AllFieldsTable` struct has the job of holding all 4 statistics kinds. 

The `evaluator::AllFieldsTableWithOutputPathOrBase` struct bundles
that with output path (XXX: what is the logic exactly with
`is_final_file`?). Those instances are specific to one of the output
formats (Excel, flamegraphs), the program evaluates separate ones
because the path syntax needs to be different for flamegraphs (to
follow the required format for the
[inferno](https://crates.io/crates/inferno) crate), and also for
flamegraphs only one kind of path is generated (and also influenced by
flamegraph-specific user options?).

The `evaluator::AllOutputsAllFieldsTable` struct bundles the separate
`evaluator::AllFieldsTableWithOutputPathOrBase` instances for all
requested output formats.

The 3 structs above (`evaluator::AllFieldsTable` /
`evaluator::AllFieldsTableWithOutputPathOrBase` /
`evaluator::AllOutputsAllFieldsTable`) are type-parameterized with a
`<Kind: AllFieldsTableKind>` type. Current such types (implementors of
`evaluator::AllFieldsTableKind`) are `SingleRunStats`, `SummaryStats`,
`TrendStats`, they are currently all empty marker types, used just to
mark the structs to clarify what kind of statistical results they
hold.

The `evaluator::AllOutputsAllFieldsTable` instance is then written to
files via its `write_to_files` method.

<!-- `evaluator::AllFieldsTable::from_log_data_tree` -->

XXXWRONG  a single choice is taken, and can specified on
the command line via `evaluator::options::FlameFieldOpt`, which was
mentioned in the "StatsField" section above; although currently the
program still evaluates the statistics for all 4 kinds first and only
then picks the chosen one for the flamegraphs.



The data structure to hold the  4 kinds of data points

`AllOutputsAllFieldsTable<Kind: AllFieldsTableKind>`


XXX  move this OUT of src/evaluator/data/ ? !   und eben why do i do it  kitschi a little  .?.

wohin? AllOutputsAllFieldsTable::from_log_data_tree is next step -- which is in src/evaluator/

3. Path index:

    After building the tree, an index over all paths is created.
    XXX (which types, and why?, what is different from the tree directly?)

### 2. 

### X. 

`AllOutputsAllFieldsTable<Kind: AllFieldsTableKind>`


### X. Creating the outputs

`StatsField<TILE_COUNT>`


#### Excel



#### Flamegraphs

The [inferno](https://crates.io/crates/inferno) library used for
generating the flamegraphs requires a format where parent scopes'
timing numbers do not include the numbers of child scopes. This is
unlike in Excel files, where the parent scope is shown with the whole
costs for that scope, regardless of which child scopes there may be,
which is both more natural when child scopes can be added to or
removed from the project over time, and also are not immediately
visible when reading the Excel file (those scope are on different rows
in the sheet). The function
[`fix_tree`](../../../src/evaluator/all_outputs_all_fields_table.rs)
converts from the child-inclusive to this child-exclusive format.

The processing is as follows:

1. First, the same code path as for Excel is used to generate an
   `AllOutputsAllFieldsTable<_>`. XXX
