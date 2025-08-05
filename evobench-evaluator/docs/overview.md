# Overview over how evobench works

The evobench project/repository maintains 3 parts: two tools,
`evobench-evaluator` and `evobench-run` (described below), and a C++
library to collect benchmarking data,
[evobench-probes](../../evobench-probes/README.md).

## evobench-evaluator

This is a tool to evaluate the log files from benchmark runs using
(currently) the [evobench-probes](../../evobench-probes/README.md)
library, and generate statistics as Excel files and flame graphs. It
needs to be given a file or files explicitly to do its work, it
doesn't know on its own about what log files might belong together. It
also doesn't execute new benchmarking runs. Run it with `--help`.

## evobench-run

This is a tool to maintain a (currently single) pipeline of queues of
benchmarking jobs that need execution now or at some particular time,
and runs those when appropriate, only ever one at the same time (to
avoid the jobs from interfering with each other and influencing the
benchmarking results). The tool has various subcommands, for polling a
repository for changes, inserting jobs, listing them, and running them
(daemon). Run it with `--help`.

It has a concept of a "key", which is all pieces of information that
influence a benchmarking run (which commit of the target project was
run, with which custom parameters, in which queuing context
(configurable), and on which machine/OS (but which is not currently
used as results are currently only stored locally)).

It currently runs the `evobench-evaluator` after each finished job run
to evaluate the results of the run and also generate summary
statistics across all runs for the same "key".

### Configuration

Currently the best documentation of the configuration file is in the
[evobench-run.ron](https://github.com/GenSpectrum/silo-benchmark-ci/blob/master/etc/evobench-run.ron)
file of the
[silo-benchmark-ci](https://github.com/GenSpectrum/silo-benchmark-ci)
repository. It also mentions where to start looking in the code if
something needs to be verified.

### Working principles

When tasking the evobench system to benchmark a particular commit,
that is creating any number of benchmarking jobs (instances of the
type `BenchmarkingJob`) via the `JobTemplate` instances declared in
the configuration file (currently it's not possible to specify a job
template on the command line). Each such job in turn is execute any
number of time again as configured. The configuration can assign
different job templates depending on the source of the commit (which
branch it was found on, or whether it came from the `evobench-run
insert` or `insert-local` commands).

`evobench-run list` and `evobench-run list-all` show one
`BenchmarkingJob` instance per line; `evobench-run list` shows how the
jobs progress, each time a job changes queue or its queue insertion
time that means a run has concluded.

To reiterate: a `GitHash` leads to 0 or more `BenchmarkingJob`s, each
of which leads to 1 or more benchmark runs, each of which leads to a
sub directory with the timestamp of the start of the run as the
directory name and holding the results for that run. The files are:

`bench_output.log.zstd`
: contents of what the target app wrote to $BENCH_OUTPUT_LOG

`evobench.log.zstd`
: contents of what evobench-probes wrote to $EVOBENCH_LOG

`single.xlsx`
: statistical results of the run, extracted from `evobench.log.zstd`

`single-*.svg`
: part of the same in flame graph form

`schedule_condition.ron`
: the queue configuration that triggered the run

`reason.ron`
: what created the job (e.g. branch name the commit was found on)

The directory above this has all the runs for the same job (actually,
all jobs with the same commit and parameters, i.e. forcing a re-issue
of a job with the same data leads to more entries here), and contains
created `summary*` files containing statistics across all the
individual runs: these represent the statistical variability across
runs (across all of them if no quoted string is in the file name, but
also separated by the value of the `situation` field in
`schedule_condition.ron` of the runs), and form the basis for change
detection or trend calculations (todo).

More details on jobs and the other entities follow below.

#### Jobs

* A benchmarking "job" is about executing a particular benchmark with
  given parameters on a given commit (i.e. should always yield the
  same timings on the same machine/OS/configuration), and is run a
  number of times, to allow to calculate a standard deviation and
  statistical significance for comparisons. It thus holds information
  that does not change over its life time--commit id and how to run it
  (environment variables and command to execute), a `reason` (why the
  job was created), and a `priority`--as well as state that changes
  with every run (a configurable `remaining_count`,
  `remaining_error_budget` and `current_boost` which can temporarily
  increase the priority, currently just for the initial run which then
  sets it back to zero). (You can see the full information on jobs via
  `evobench-run list -v`.)

* One commit leads to the insertion of any number of jobs as
  statically determined in the configuration, with different
  parameters (priority values, target, custom variables), depending on
  where the commit was found (which branch, or command line).

* Only one job is ever *running* at any one time, to ensure a run has
  deterministic control (i.e. that there are no other jobs influencing
  the results).

#### Queues and pipelines

* A job is always being held by one queue at any time, but can be
  passed on by the queue. There are 3 reasons it can be passed on: it
  is finished (`remaining_count` dropped to zero), it failed too many
  times (`remaining_error_budget` dropped to zero), or the queue
  decides that the job should be processed by another queue. There are
  different kinds of queues (currently `Immediately`,
  `LocalNaiveTimeWindow`, and `GraveYard`) with different rules,
  configurable for some kinds. They are working on the oldest job they
  have first (first in, first out), except job priorities dictate that
  higher-priority jobs are worked on first (`GraveYard` queues do not
  work on their jobs).

* If a job execution fails, the job is always re-inserted into the
  same queue it was run from (but with the lowered
  `remaining_error_budget`). This means, its insertion time is updated
  (i.e. it will be scheduled after other jobs with the same priority
  in the queue; this gives potentially non-failing jobs a chance to
  yield results sooner).

* Queues are (currently) implemented as a directory with files, one
  per job. By default each queue has a subdirectory under
  `~/.evobench-run/queues/`. Entries are inserted with the current
  hi-res time stamp (plus process id and a process-local counter to
  disambiguate) as the file name, and are serialized `BenchmarkingJob`
  structs.

* Queues are lined up in a pipeline: a job enters the first queue of a
  pipeline, then when the queue holding it decides to move it on (for
  the 3rd reason mentioned above), it is inserted into the next queue
  in the pipeline. If there is no next queue, the job is dropped. Note
  that some queue kinds can (be configured to) hold jobs (until the
  job's `remaining_count` or `remaining_error_budget` is zero) instead
  of passing them on after one execution.

* Besides the queues in the pipeline, there are also two special
  optional configurable queues: one to catch all jobs that are
  finished, and one to catch all jobs that error out. If either is
  `None`, jobs will be dropped instead in that case.

* Whenever a job execution is finished, `evobench-run run daemon` tries to select
  the next job to execute, if any, by first filtering the queue pipeline
  for the queues that are executable at the given time, then selecting
  the job(s) with the maximum total priority, where the total priority
  is the sum of the job's `priority` and `current_boost` fields and
  the queue's own priority, which is either as configured for the
  queue in the configuration (e.g. `priority` field for
  `LocalNaiveTimeWindow`), or the queue's default, which is 0 for
  `Immediately` and 1.5 for `LocalNaiveTimeWindow`
  (`TIMED_QUEUE_DEFAULT_PRIORITY` constant in the code). Of all the
  jobs with the same maximum priority, the oldest in the earliest
  queue (i.e. the one appearing closest to the top in the
  `evobench-run list` view) is chosen.

* Queues have (at least currently, and it seems useful to keep it that
  way) no state other than the jobs that they contain. (Jobs however
  have, as mentioned above, changing state over their life time.)
  This is why queues can't offer e.g. a parameter to specify that they
  should process a job 5 times; the only thing they can do is either
  process a job once or infinitely, or, if that really seems useful,
  have a condition like "process jobs until their `remaining_count` is
  <= 5". But it seemed more direct (easier to understand and see via
  `evobench-run list`) to just have e.g. 5 `Immediately` queues in a
  row in the pipeline to run a job 5 times. (If really needed, my
  suggestion would be to add another stateful field to *jobs*, rather
  than to queues, that contains the number of runs in the current
  queue.)

#### Working directories

When a job is executed, that is of course done inside a clone of the
target project repository (a working directory, in Git parlance).

To avoid the overhead of cloning that repository and rebuilding it,
and for the targetted program to allow caching cachable data across
runs, those working directories are kept around. Also, since the
queues can contain jobs for multiple commit ids, and with different
parameters, but those jobs need multiple executions, evobench-run is
keeping a pool of working directories around (configurable in
`capacity` under `working_directory_pool`), and tracks which directory
has which commit id checked out (and perhaps more data soon), to then
try and allocate job runs to the working directory that is closest to
the job for minimizing the overhead.

The default location for this pool is at
`~/.evobench-run/working_directory_pool/`. The same directory also
contains the log files from running benchmarking jobs; they start with
`$n.output_of_benchmarking_command*`, where `$n` is the number of the
working directory. There are also `$n.error*` files in case a job
failed, and soon `$n.status` files to store the status of a working
directory.
