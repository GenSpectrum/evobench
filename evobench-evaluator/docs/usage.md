
## General

Note that unlike in most unix command line programs, the position of
options relative to subcommands matters. E.g.

    evobench-run -v list

shows the default terse listing, while enabling debugging information,
e.g. what path the configuration file is read from, whereas

    evobench-run list -v 

shows the detailed listing. The following enables verbosity for both purposes:

    evobench-run -v list -v 

To run the daemon, you may want to enable some options and redirect
the output (proper built-in daemonization may come in the future):

    RUST_BACKTRACE=1 nohup evobench-run -v run daemon --restart-on-upgrades

