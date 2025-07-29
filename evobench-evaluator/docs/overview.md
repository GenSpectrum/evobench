# Overview over how evobench works

## evobench-evaluator

TODO

## evobench-run

### Working principles

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
  sets it back to zero).

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

