# Evobench: the evobench-probes part

This is the library to record benchmarking data from with in target
applications written in C++.

## How to add the probes infrastructure

For simplicity (couldn't get the library to work as a Conan package),
how the library is added to the target application is by copying the
library files into the target application's repository. There is a
script for doing this; example for
[LAPIS-SILO](https://github.com/GenSpectrum/LAPIS-SILO/):

    git clone https://github.com/GenSpectrum/evobench/
    git clone https://github.com/GenSpectrum/LAPIS-SILO/
    cd LAPIS-SILO
    ../evobench/evobench-probes/bin/add-include-and-src

## How to add probes

  - `#include` [evobench/evobench.hpp](include/evobench/evobench.hpp)

  - Place `EVOBENCH_SCOPE("module", "action")` before code you want to
    benchmark. Both arguments have to evaluate to a C string
    literal. The two arguments are joined statically with a `|`
    character inbetween (maybe this is somewhat pointless, the idea is
    to encourage a hierarchical naming structure). This probe places
    an object with a destructor and records both at construction and
    destruction time.

  - Place `EVOBENCH_POINT("module", "action")` if you want a single
    time measurement; but currently evobench-evaluator does not do
    anything useful with these (XX right?, todo).

  - Place `EVOBENCH_KEY_VALUE("key", value)` to log a runtime value
    (`value` needs to evaluate to a string), without any timings;
    evobench-evaluator adds these as a pseudo scope into the tree (and
    automatically closes the pseudo scope when getting the closing
    measurement of the surrounding `EVOBENCH_SCOPE` probe.

All of these are macros that can be fully compiled out by defining
`NO_EVOBENCH`; but when compiled in (the default), their cost is
minimal (just a boolean check for every logging point, and one pointer
width of stack overhead for an `EVOBENCH_SCOPE`) as long as the
`EVOBENCH_LOG` environment variable is unset. Logging is activated by
setting that variable to the desired path to write the log to.

## How to do benchmark runs

  - Set the `EVOBENCH_LOG` environment variable to a path into an
    existing directory. Using a directory below `/dev/shm/` (tmpfs,
    shared memory filesystem instead of disk-based real file system)
    leads to a measurable performance benefit. Of course you have to
    make sure that your benchmarking run doesn't generate files so
    large that they fill up available RAM, because that would lead to
    worse slowdowns than using a real file system. Also, move away the
    file (compress it with zstd while doing so!) after every run.

  - Look at the Makefile/script based infrastructure XX for how to do
    it practically.

## Implementation details

  - To avoid accidental concurrent runs of multiple program instances
    with the same log file (as per `EVOBENCH_LOG` environment
    variable), the library takes a lock via `flock`(2). If the library
    detects that the file is already locked, it exits the program with
    an error.

  - To avoid lock contention amongst the threads on the output
    filehandle, and since ordering of events only needs to be upheld
    within a thread, not across threads, the library implements its
    own buffering approach, with each thread getting its own
    buffer. Those buffers are flushed when full or then the
    corresponding thread (or the process) ends. To properly end the
    program it's important to let the program shut down cleanly (C++
    destructors of thread-local variables need to be called);
    e.g. LAPIS-SILO does that when receiving SIGINT, but *not* when
    receiving the default signal that the `kill` bash built-in uses,
    SIGTERM. If the program does not shut down cleanly,
    evobench-evaluator detects the missing scope / thread / program
    end messages and reports an error.

  - Since flushing those buffers need actual IO, the flush action also
    logs the timings; but currently evobench-evaluator ignores those.
    It could use them to judge the cost of the flushing calls and
    then subtract those from the other timings, to virtually eliminate
    this overhead; the overhead is currently deemed low enough to not
    have warranted carrying out this work.

  - NDJSON has been chosen as the format for the log files. Some care
    has been taken to make JSON serialization fast; the `SLOW(expr)`
    macro in [src/evobench/evobench.cpp](src/evobench/evobench.cpp) is
    an exception and allocates memory, and could be a low-hanging
    fruit for optimization.

    The evobench-probes library writes the files uncompressed. But
    evobench-evaluator transparently decompresses files with `.zstd`
    suffix; the idea is to compress the files for archival.
