# Evobench software benchmarking and job scheduling system: tooling

This Rust crate contains the tooling part of the "Evobench" system. If
you haven't, first read the [main project page](../README.md) for an
overview of the larger picture.

There are multiple tools, decribed below. The main tool is the
`evobench` tool--you will normally only directly interact with
that. The tools are using subcommands, sometimes (especially in the
`evobench` tool) multiple levels deep. See
[subcommands](docs/subcommands.md) for more information on this.

## evobench-eval

This is a tool to evaluate the log files from benchmark runs using
(currently) the [evobench-probes](../evobench-probes/README.md)
library, and generate statistics as Excel files and flame graphs. It
needs to be given a file or files explicitly to do its work, it
doesn't know on its own which log files belong together. It also
doesn't execute new benchmarking runs. Run it with `--help`. Usually
this is only run as a helper by the `evobench` tool.

## evobench

This is a tool to maintain a (currently single) pipeline of queues of
benchmarking jobs that need execution now or at some particular time,
and runs those when appropriate, only ever one at the same time (to
avoid the jobs from interfering with each other and influencing the
benchmarking results). The tool has various subcommands:

<dl>
  <dt>insert</dt>

  <dd>Insert jobs manually into the benchmarking queue pipeline, either by
  giving one (or more) commit id(s) and a reference to a set of job
  templates from the configuration file, or by giving a path to a job
  description file</dd>
  
  <dt>list</dt>
  <dd>List the currently scheduled and running jobs.</dd>

  <dt>list-all</dt>
  <dd>Show the list of all inserted jobs, including already processed ones.
    </dd>

  <dt>poll</dt>
  <dd>Insert jobs for new commits on branch names configured in the
    config option <code>remote_branch_names_for_poll</code>. Usually
    run as a daemon via the `daemon` subcommand.</dd>

  <dt>run</dt>
  <dd>Run the existing jobs; this takes a lock or stops with an error
    if the lock is already taken. Usually as a daemon via the `daemon`
    subcommand.</dd>

  <dt>wd</dt>
  <dd>Handle working directories: entering one, reading the last job
  log, marking to save from deletion, deleting or recycling back into
  use, cleanup.</dd>
  
  <dt>status</dt>
  <dd>Show status information about the whole evobench system,
   including the two daemons (poll and run), and configured paths that
   one might want to inspect manually.</dd>
   
</dl>

Besides the command line use, the `evobench run` subcommand also
generates some HTML files into the output directory, for serving as
static files via a web server. (The web interface may be extended over
time.)

### Benchmarking job key 

The runner has a concept of a "key", which represents all pieces of
information that influence a benchmarking run:

- which commit of the target project is tested,

- with which target name (projects can define multiple benchmarking
  targets, e.g. "api" or "preprocessing")

- with which custom parameters, 

- and in which queuing context (configurable, e.g. "night" and "day",
  to distinguish results if the benchmarking host has some other loads
  during the day that might influence the results)

Host, OS, and compiler versions could also be made part of the key,
but currently those are ignored and the assumption is that all results
come from the same such context.

It currently runs the `evobench-eval` after each finished job run to
evaluate the results of the run and also generate summary statistics
across all runs for the same "key". Tracking graphs and trends across
the commit history for the "key" summaries will probably be the next
feature that will be added.

![](docs/evobench-run-list-2025-11-25.png "Example output of `evobench list`")

### Shell completions

The `evobench` tool has a `completions` subcommand that outputs shell
files that can be included from the shell startup files to get tab
completion for options and subcommands. E.g.:

    evobench completions bash > ~/.bash_completions_evobench

then include that file from `.bashrc`.

### Configuration

To specify the many details about job running behaviour, `evobench`
needs a configuration file. Unless given the `--config` argument, it
expects it at `.evobench.*`, with various suffix alternatives (run
`evobench config-formats` for the supported ones; RON is recommended
as it is the most expressive of those formats (it allows to list the
names of the types being filled in, distinguishes between fields and
dictionaries, and has explicit tuple syntax), and is also close to
Rust syntax).

Currently the best documentation of the configuration file is in the
comments of the
[evobench.ron](https://github.com/GenSpectrum/silo-benchmark-ci/blob/master/etc/evobench.ron)
file of the
[silo-benchmark-ci](https://github.com/GenSpectrum/silo-benchmark-ci)
repository. The file also mentions at the top where to start looking
in the code if more details are needed.

### Environment

If the target application requires environment variables to be present
to compile or run (e.g. for a loaded Python virtualenv, or a `PATH`
that includes directories to `uv` or `cargo`), there are two choices:

1. Set those variables before starting the runner. `evobench run
   daemon start` retains the current environment in the daemon process. Note:
   
   * `evobench run daemon soft-restart` has the daemon restart itself when time comes, it will not be forked from the current shell and hence *not* take on environment changes. Use `evobench run daemon restart` instead.
   * cron jobs do not start with the login environment; instead, you have to set variables in the crontab. See `man 5 crontab`.

1. Alternatively, you can configure shell code in Bash syntax to run
   before executing the target program, in the toplevel
   `target_pre_exec_bash_code` configuration field, and/or the
   `targets.benchmarking_command.pre_exec_bash_code` fields.

### Working principles

When tasking the evobench system to benchmark a particular commit (via
`evobench insert` or `evobench poll`), that is creating any number of
benchmarking jobs (instances of the type `BenchmarkingJob`) via the
`JobTemplate` instances declared in the configuration file (currently
it's not possible to specify a job template on the command line). Each
such job is executed the number of time again as configured in
`benchmarking_job_settings.count`. The configuration then assigns a
set of templates to branch names; adding commits from a branch uses
the specified templates. This allows to configure special benchmark
trigger branches on the upstream GitHub repository, force-pushing to
them will trigger new benchmarking jobs (if the `evobench poll daemon`
is running).

`evobench list` and `evobench list-all` show one `BenchmarkingJob`
instance per line. `evobench list` shows how the jobs progress: each
time a job changes queue or its queue insertion time that means a run
has concluded.

Results of benchmarking runs are stored below the directory configured
in `output_dir.path`, with a path that is made up from the target
name, the custom variables, the commit id, and the run timestamp. Example:

    ~/silo-benchmark-outputs/api/CONCURRENCY=120/DATASET=SC2open/RANDOMIZED=1/REPEAT=1/SORTED=0/44b58f2b51a60c3df14a35109db66b0fb37db623/2026-02-22T18:53:25.819835794+01:00
    #   ^ output_dir.path    ^target_name     ^----^----------------^---custom variables-----^   ^commit_id                              ^ run timestamp

To reiterate: a git commit (`GitHash` type in the code) leads to 0 or
more `BenchmarkingJob`s, each of which leads to a configured number of
benchmark runs, each of which leads to a sub directory with the
timestamp of the start of the run as the directory name and holding
the results for that run. The files are:

`bench_output.log.zstd`
: the contents of what the target app wrote to the path in
  `$BENCH_OUTPUT_LOG` (could be an application log file)

`standard.log.zstd`
: what the target app wrote to stdout and stderr (and a head with all
  the job parameters in YAML format; the type `CommandLogFile` in
  [run/command_log_file.rs](src/run/command_log_file.rs) can extract
  this information back from those files)

`evobench.log.zstd`
: the performance data (the contents of what the evobench-probes
  library automatically wrote to the path in `$EVOBENCH_LOG`)

`schedule_condition.ron`
: the queue configuration that triggered the run

`reason.ron`
: what created the job (e.g. branch name the commit was found on)

`single.xlsx`
: the statistical results of the run, extracted from
  `evobench.log.zstd`

`single-*.svg`
: a part of the same statistical data in flame graph form

The directory one level higher up has all the runs for the same job
(as well as other jobs issued with the same commit and parameters,
which normally doesn't happen but can be triggered by using `evobench
insert .. --force`). It contains derived `summary*` files containing
statistics across all the individual runs: these represent the
statistical variability across runs (across all of them if no quoted
string is in the file name, but also separated by the value of the
`situation` field in `schedule_condition.ron` of the runs), and form
the basis for change detection or trend calculations (todo).

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
  `evobench list -v`, or by looking at the files in the queues
  directories, which by default are below `~/.evobench/queues/`.)

* One commit leads to the insertion of any number of jobs as
  statically determined in the configuration, with different
  parameters (priority values, target, custom variables), depending on
  where the commit was found (which branch, or command line).

* Only one job is ever running at any one time, to ensure that a
  benchmarking run has deterministic control (i.e. that there are no
  other jobs influencing the results). This is implemened via flock
  (`man 2 flock`) on the directory for the run daemon (flock ensures
  that there are never stale locks, when all processes forked from a
  run are killed, the lock is automatically gone). (Note that flock
  locks are used in some other places, too: one by `daemon start`; one
  when mutating a working directory pool; one for each individual
  queue entry when worked on.)

#### Queues and pipelines

* A job is always being held by one queue at any time, but can be
  passed on to another queue. There are 3 reasons it can be passed on:
  it is finished (`remaining_count` dropped to zero), it failed too
  many times (`remaining_error_budget` dropped to zero), or the queue
  decides that the job should be processed by another queue. There are
  different kinds of queues (currently `Immediately`,
  `LocalNaiveTimeWindow`, and `Inactive`) with different rules,
  configurable for some kinds. They are working on the oldest job they
  have first (first in, first out), except job priorities dictate that
  higher-priority jobs are worked on first (`Inactive` queues do not
  work on their jobs).

* If a job execution fails, the job is always re-inserted into the
  same queue it was run from (but with the lowered
  `remaining_error_budget`). This means, its insertion time is updated
  (i.e. it will be scheduled after other jobs with the same priority
  in the queue; this gives potentially non-failing jobs a chance to
  yield results sooner).

* Queues are implemented as a directory with files in JSON format, one
  per job. By default each queue has a subdirectory under
  `~/.evobench/queues/`. Entries are inserted with the current hi-res
  time stamp (plus process id and a process-local counter to
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

* Whenever a job execution is finished, `evobench run daemon` tries to select
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
  `evobench list` view) is chosen.

* Queues have (at least currently, and it seems useful to keep it that
  way) no state other than the jobs that they contain. (Jobs however
  have, as mentioned above, changing state over their life time.)
  This is why queues can't offer e.g. a parameter to specify that they
  should process a job 5 times; the only thing they can do is either
  process a job once or infinitely, or, if that really seems useful,
  have a condition like "process jobs until their `remaining_count` is
  <= 5". But it seemed more direct (easier to understand and see via
  `evobench list`) to just have e.g. 5 `Immediately` queues in a
  row in the pipeline to run a job 5 times. (If really needed, my
  suggestion would be to add another stateful field to *jobs*, rather
  than to queues, that contains the number of runs in the current
  queue.)

#### Working directories

When a job is executed, that is done inside a clone of the target
project repository (a working directory, in Git terminology). The
application must provide some kind of `Makefile` or script that builds
the app and carries out a benchmarking run, all in one invocation (for
more see "Benchmarking entry point" below).

To avoid the overhead of cloning that repository and rebuilding it,
and for the targetted program to allow caching cachable data across
runs, those working directories are kept around. Also, since the
queues can contain jobs for multiple commit ids (and with different
parameters), but those jobs need multiple executions, evobench is
keeping a pool of working directories around (configurable in
`working_directory_pool.capacity`), and tracks which directory has
which commit id checked out, to then try and allocate job runs to a
working directory that has already been used for the same commit id.

The default location for this pool is at
`~/.evobench/working_directory_pool/`. The same directory also
contains the log files from running benchmarking jobs; they start with
`$n.output_of_benchmarking_command*`, where `$n` is the number of the
working directory--those files are moved to the output directory for
successful runs, but (currently) stay put if there were errors (the
thought being that only successful runs should show up in the output
directory tree). There are also `$n.error*` files in case a job
failed, and `$n.status` files to store the status of a working
directory.

Working directories have numeric ids; in the user interface they are
prefixed with "D" ("d" is also accepted) to disambiguate from other
numbers.

The `evobench wd` subcommand should be used to interact with working
directories. It knows how to change the status to prevent a directory
that one is inspecting from being deleted, to avoid accidentally
manually working in a directory that is in use or could be used, and
it knows how to signal to the daemon when status changes
happen. `evobench wd enter` also sets up the whole environment that
was effective in the last run in that dir; and `evobench wd log`
automatically finds the last log file with errors for the given
directory id.

### Benchmarking entry point

The application needs to provide a means to execute a build and
benchmarking run in one command invocation. What command, arguments
and subdirectory are all configurable. Multiple different sets of
command/argument/subdirectory can be configured in `targets`, with
`target_name` being used to identify them.

For an example setup, see the one in the
[LAPIS-SILO](https://github.com/GenSpectrum/LAPIS-SILO/) database
system, in the
[benchmarking](https://github.com/GenSpectrum/LAPIS-SILO/tree/main/benchmarking)
subfolder--matching the
[evobench.ron](https://github.com/GenSpectrum/silo-benchmark-ci/blob/master/etc/evobench.ron)
already mentioned above. That folder contains a `Makefile` with two
possible targets, `preprocessing` and `api`, that builds the
application, preprocesses the dataset, then for `api` runs the
application with that dataset, using
[api-query](https://github.com/pflanze/api-query) to run queries
against it and record checksums for the results. The runs can be
parameterized via the custom variables as configured in the
evobench.ron.

### Results

Currently, there isn't much in terms of a web interface to acces the
results; they can be served statically (via e.g. nginx), and there are
a few html files in the top level of the output folder to find one's
way around them ([example for
LAPIS-SILO](https://silo-benchmarks.genspectrum.org/)). This area
could use more work.

## Other tools

The other tools are less directly useful:

* `evobench-migrate`: Database migration for evobench: update storage
  format for jobs in queues. Run this when you're getting
  deserialisation errors from `evobench`, or when you know that the
  data structures have changed and will cause errors.

* `evobench-util`: various supplemental functions, e.g. to re-process
  output files (useful when the way how derived files are generated
  changes, or when they get lost/damaged), and some functionality to
  help interactive testing during development

* `evobench-ndjson`: currently a way specific to LAPIS-SILO to work
  with API query sets, but could be generalized. Allows to cluster
  queries according to similarity.

* `jobqueue`: a generic job queue utility, based on the job queue
  library used in `evobench`. Perhaps useful outside `evobench`?

* `try-bench_tmp_dir`, `trying-git`: used for interactive testing
  during development.


## Account setup

It can be useful to have a dedicated user account for evobench on the
host where it runs, to interact with the system on the command line.

The LAPIS-SILO-specific
[silo-benchmark-ci](https://github.com/GenSpectrum/silo-benchmark-ci/)
repository contains files to help with such an account, including
welcome message and `help-benchmarking` command to enable users
without in-depth knowledge carry out basic work.

## Debugging problems

* Know where to look for the log files. `evobench status` shows the
  log directory paths. `evobench run daemon logf` follows the newest
  logfile. `evobench wd log D..` shows the last log for a working
  directory for which a run failed.

* To get even more information about what the tools are doing, use the
  `--verbose` or even more verbose `--debug` options (or alternatively
  `--log-level info|debug`). E.g. `evobench run daemon --debug
  restart` to restart that daemon to log more information. (The run
  daemon runs in `--verbose` mode by default, already. But `--debug`
  gives yet more information.)

* If a tool stops with an error, and you want to know where in the
  code the error happens, rebuild the tool in debug mode (`cargo
  build` *without* `--release`, then run the binary from
  `target/debug/`) and run it with `RUST_BACKTRACE=1`. (You can also
  `cargo run` to build and run, as in `RUST_BACKTRACE=1 cargo run
  --bin evobench --`.)
