
## General

Note that unlike in most unix command line programs, the position of
options relative to subcommands matters. E.g.

    evobench-jobs -v list

shows the default terse listing, while enabling debugging information,
e.g. what path the configuration file is read from, whereas

    evobench-jobs list -v 

shows the detailed listing. The following enables verbosity for both purposes:

    evobench-jobs -v list -v 

To run the daemon, you may want to enable some options and redirect
the output (proper built-in daemonization may come in the future):

    source ~/venv/bin/activate
    RUST_BACKTRACE=1 nohup evobench-jobs -v run daemon --restart-on-upgrades

