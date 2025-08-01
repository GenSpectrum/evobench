# Overview over how evobench works

## evobench-evaluator

TODO

## evobench-run

### Working principles

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

#### Queues and pipelines

* Queues are (currently) stored as files in a directory. By default
  each queue has a subdirectory under
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

* Whenever a job is finished, `evobench-run run daemon` tries to select
  the next job to run, if any, by first filtering the queue pipeline
  for the queues that are runnable at the given time, then selecting
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
`$n.output_of_benchmarking_command*`, where $n is the number of the
working directory. There are also `$n.error*` files in case a job
failed, and soon `$n.status` files to store the status of a working
directory.
