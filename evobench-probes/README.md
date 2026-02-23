# Evobench: the evobench-probes part

This is a library to record benchmarking data from within target
applications written in C++, in the native "evobench" format (which is
also currently the only supported format). See "Implementation
details" below for details on the format.

## How to add the probes infrastructure

For simplicity (I couldn't get the library to work as a Conan
package), the library is added to a target application by copying the
library files into the target application's repository. There are two
scripts for doing this; example for
[LAPIS-SILO](https://github.com/GenSpectrum/LAPIS-SILO/) (but this has
already been done in the past--you could check out
4902df038caef570f0f9e50eacd91c518a18cdae from before if you want to
try it for real):

    git clone https://github.com/GenSpectrum/evobench/
    git clone https://github.com/GenSpectrum/LAPIS-SILO/
    cd LAPIS-SILO
    ../evobench/evobench-probes/bin/add-include-and-src-to-src

## How to add probes

  - `#include` [evobench/evobench.hpp](include/evobench/evobench.hpp)

  - Place `EVOBENCH_SCOPE("module", "action")` before code you want to
    benchmark. Both arguments have to evaluate to a C string
    literal. The two arguments are joined statically with a `|`
    character inbetween (maybe this is somewhat pointless, the idea is
    to encourage a hierarchical naming structure). This probe places
    an object with a destructor and records both at construction and
    destruction time. If the code is a hot loop for which log
    generation causes too large log files and too much slow down, use
    `EVOBENCH_SCOPE_EVERY(n, "module", "action")` instead, with n
    being a divider; i.e. pass 1000 to only log every 1000th
    evaluation.

  - Place `EVOBENCH_POINT("module", "action")` if you want a single
    time measurement; but currently evobench-eval does not do anything
    useful with these (todo: do something, or remove the feature).

  - Place `EVOBENCH_KEY_VALUE("key", value)` to log a runtime value
    (`value` needs to evaluate to a string), without any timings;
    evobench-eval adds these as a pseudo scope into the tree (and
    automatically closes the pseudo scope when getting the closing
    measurement of the surrounding `EVOBENCH_SCOPE` probe.

All of these are macros that can be fully compiled out by defining
`NO_EVOBENCH`; but when compiled in (the default), as long as the
`EVOBENCH_LOG` variable is not set, their cost is still minimal (just
a boolean check for every logging point, and one pointer width of
stack overhead for an `EVOBENCH_SCOPE`). Logging is activated by
setting that variable (`evobench run` does that) to the desired path
to write the log to.

See [example/](example/) for a simple instrumentation example. Run
`make run` to build and run the program, producing a `bench.log`
output file. `make eval` to evaluate the `bench.log` file to an Excel
file with statistics and a set of SVG files with flamegraphs.

## How to do benchmark runs

  - Set the `EVOBENCH_LOG` environment variable to a path into an
    existing directory. On Linux, using a directory on a tmpfs
    filesystem (`dev/shm` on older distros, systemd on newer Debian
    cleans that out thus use `/tmp` instead or set up a partition
    yourself) leads to a measurable performance benefit (but you have
    to make sure that your benchmarking run doesn't generate files so
    large that they fill up available RAM, because that would lead to
    worse slowdowns than using a real file system). 
    
  - Normally you will run the application via `evobench run`, which
    selects a tmpfs automatically if it finds one, and sets the
    `EVOBENCH_LOG` variable (plus other variables; see
    [evobench-tools/README.md](../evobench-tools/README.md) for
    further info).

  - For an example how to control an application for benchmarking via
    `evobench run`, have a look at the
    [benchmarking](https://github.com/GenSpectrum/LAPIS-SILO/tree/main/benchmarking)
    directory in the
    [LAPIS-SILO](https://github.com/GenSpectrum/LAPIS-SILO/) project.

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
    evobench-eval detects the missing scope / thread / program
    end messages and reports an error.

  - Since flushing those buffers need actual IO, the flush action also
    logs the timings; but currently evobench-eval ignores those.
    It could use them to judge the cost of the flushing calls and
    then subtract those from the other timings, to virtually eliminate
    this overhead; the overhead is currently deemed low enough to not
    have warranted carrying out this work so far.

  - [NDJSON](https://en.wikipedia.org/wiki/Ndjson) has been chosen as
    the format for the log files. Some care has been taken to make
    JSON serialization fast; the `SLOW(expr)` macro in
    [src/evobench/evobench.cpp](src/evobench/evobench.cpp) is an
    exception and allocates memory, and could be a low-hanging fruit
    for optimization.

    The evobench-probes library writes the output file uncompressed. But
    `evobench run` compresses them via zstd after a run has finished,
    and `evobench-eval` transparently decompresses those files.

    The writer is efficient enough to not slow down the app more than
    a couple % in normal use, and the reader (in `evobench-eval`) can
    process 1 GB of uncompressed log data in under a second on a
    machine with 16+ cores (zstd decompression costs another ~0.5
    seconds).

    The possible messages and their types are defined in the parser in
    Rust at
    [evaluator/data/log_message.rs](../evobench-tools/src/evaluator/data/log_message.rs).

  - class `evobench::Output` outputs to a file, it holds the bare fd
    and a mutex (since while an individual write system call is
    atomic, writing a buffer might require multiple writes). The
    buffer is per thread, in class `evobench::Buffer`. The main thread
    logs on init and destruction of Output, and allocates a temporary
    buffer for the purpose each time.
    
    The code is written in a low level style and including C headers
    to keep it close to C, to potentially make it compatible with C
    (with GCC's extension for destructors).

