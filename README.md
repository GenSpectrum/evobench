# The Evobench software benchmarking and job scheduling system

The aim of Evobench is to measure performance of software under
development and catch performance regressions or track steady
improvements over time. The goal is not to find bottlenecks (for which
tools like [Linux
perf](https://en.wikipedia.org/wiki/Perf_%28Linux%29) work well), but
to automatically track changes across commits and ensure that
development doesn't accidentally make subsystems slower over time.

To do this, the software is explicitly instrumented with probes: these
can be named independently of functions, an be moved between functions
when refactoring. The idea is to instrument particular subsystems or
parts of the application which are known to stay around in some form
over time, and have well-understood purposes which are interesting to
track for a longer development period. Currently, Evobench contains a
library for C++ in the [evobench-probes](evobench-probes/README.md)
directory. It is a very small library, with usually negligible cost
when unused (and can be completely disabled at compile time), that
records performance relevant data when an environment variable is set
(real, cpu and sys timings, context switches, there are ideas to
extend it for measuring memory allocations).

Then there is a set of tools in the
[evobench-tools](evobench-tools/README.md) directory, written in Rust:

- `evobench-eval` evaluates the traces, producing Excel files with
  statistics and SVG files with flame graphs;
  
- `evobench` is a job scheduling system to execute benchmarking runs
  of the target software:
  
    - `evobench poll` tracks Git repositories for new commits and
      creates new benchmarking jobs that it inserts into a set of job
      queues.
    - `evobench run` executes jobs from those job queues, as they come
      in; it supports scheduling priorities, re-runs on errors, re-use
      of working directories for efficiency, scheduling at particular
      times of day; it picks up a job, runs it with the parameters
      particular to that job (commit id, but also a target name and
      custom parameters), and afterwards compresses the output files
      and runs `evobench-eval` to produce extracts.
    - It also offers a number of subcommands to work with the job
      system from the command line (`evobench insert` to insert a job
      manually, `evobench list` to list the contents of the queues,
      `evobench list-all` to list all jobs ever run, `evobench wd` to
      list and interact with the dynamically created and deleted
      working directory where jobs are executed)
    - It has the beginnings of a web interface to the same information
      (e.g. see the live view of the [LAPIS-SILO benchmarking job
      queue](https://silo-benchmarks.genspectrum.org/list.html))
    - The best approach to benchmarking is probably using captured
      live data (e.g. real logged API queries). But the job scheduling
      system can also be used for some other purposes, e.g.:
        - verification of API results for continued correctness via
          checksums, by logging with the `iter --log-csv` feature of
          the [api-query](https://github.com/pflanze/api-query) tool
          and verifying the log via its `api-query-log` companion tool;
        - stress-testing the application and verifying that it doesn't
          crash or produce invalid results;
        - generating binaries for publishing;
        - running a normal test suite (with manually prepared data)
        
    Unlike systems like GitHub CI, the evobench job runner can run on
    systems that are more powerful than what might be available there,
    and can run directly on the host machine, making sure to yield
    precise performance measurement undisturbed by other services or
    tenants on the host. It also retains the working directories where
    failed jobs were run, and allows to directly inspect them via ssh
    and shell tools (`evobench wd enter ..` runs a shell with the
    complete environment that a job was run in).

## Sponsor

This software was originally written for and is used for benchmarking
the [LAPIS-SILO](https://github.com/GenSpectrum/LAPIS-SILO/) database
system maintained by the [Computational Evolution group at ETH
Zurich](https://bsse.ethz.ch/cevo). "Evobench" stands for
"benchmarking system for software evolution".
