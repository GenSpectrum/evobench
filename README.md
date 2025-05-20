# The Evobench benchmarking infrastructure 

This software is currently used for benchmarking
[cEvo](https://bsse.ethz.ch/cevo)'s
[LAPIS-SILO](https://github.com/GenSpectrum/LAPIS-SILO/) database.

It consists of multiple parts:

 -  [evobench-probes](evobench-probes/README.md), a small library in C++ to
    record performance relevant data (real, cpu and sys timings,
    context switches, it might be feasible to extend for measuring
    memory allocations). Probes are explicitly to the source code of
    the program to be benchmarked.  Probes are primarily dynamically
    scoped, and measure from their place of insertion to the end of
    the scope (via C++ destructors), but additional point probes and
    runtime info logging statements are available. The probes/info
    statements are represented with CPP macros. By default they are
    compiled in, but disabled at runtime and have a minimal overhead
    that way. By setting the `EVOBENCH_LOG` environment variable to a
    file path, the library becomes active and retrieves and logs the
    data.

 -  [evobench-evaluator](evobench-evaluator/), a program written in
    Rust that can parse the log files written by evobench-probes,
    builds a tree representation of the benchmarking calls (probe
    calls) according to their dynamic nesting, and generates
    statistics across all the measurements for all paths in the
    tree. The output can be written as CSV tables (Excel soon?). Also,
    it can read multiple log files and calculate differences (results
    of different target program versions) and statistics (multiple
    results of the same target program version) (XXX to be finished &
    published).

 -  scripts / Makefile to get all the dependencies and do benchmarking
    runs on SILO. (XXX to be published.)

