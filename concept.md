

    testname/commit/hostname/run-start-time/context/scope-name.csv # one row per cal

Example:

    parallel_test/7245a11b85e88f3f13805405e7f7035846673c8f/gs-dev-1/1742975565.123/requestid=curl8729/


C++ part:

- class evobench::Output: to a file, holds the bare fd, has a mutex for writing, holds no buffer
- class evobench::Buffer: each thread allocates their own in thread init, lets go in thread exit, uses a std::string to hold the buffered data. For performance, output is done by appending to the string directly, no stream abstraction is used (and std::ostringstream would not be suitable as it allocates/deallocates the backing storage on creation/destruction and has no way to clear it).


- main thread logs on init and destruction of Output, and allocates a temporary buffer for the purpose each time.


Metadata and Start are separate so that the reader can change the parser version before reading the possibly changed Metadata. Start must never change structure, unlike what follows.
