# CLI tool subcommands

The command line tools are using subcommands (via
[Clap](https://crates.io/crates/clap)'s Subcommand feature), sometimes
in a hierarchy multiple levels deep, to group functionality.

Each level has its own help, via `--help` (sometimes just `help`
works, too)--so be sure to check through the levels, as there is no
single help page that lists all the areas at once (although perhaps
such a feature could be implemented?)

Subcommands also have their own options applicable to them--they are
specified after the subcommand name. Thus with the Evobench tools,
unlike with many unix tools, it matters where options are placed. E.g.

    evobench --debug run daemon start

does not have the same meaning as:

    evobench run daemon --debug start

In the first case, debugging is turned on globally for all subcommands
and contexts, but in the second case, only inside the daemon context
(meaning, only once the daemon log file is opened, with all debugging
info being redirected there).

As another example:

    evobench -v list

shows the default terse listing, while enabling debugging information,
e.g. what path the configuration file is read from, whereas

    evobench list -v 

shows the detailed listing. The following enables verbosity for both purposes:

    evobench -v list -v 

## Enumerations that are not subcommands

Note that in some cases there are string arguments from a selection of
valid values that are not subcommands; the `start` and `stop` values
from the `daemon` subcommand above are examples. Another is `evobench
insert branch`, which with `-h` added to the end will show you that it
expects a `LOCAL_OR_REMOTE` argument, which can be the `local` or
`remote` string. This doesn't constitute a subcommand, just an
enumeration argument; there are further arguments after it that belong
to `insert branch`. `evobench insert templates -h` also has the same
`LOCAL_OR_REMOTE` argument, but now in a different context: it and
anything after it belonging to the `insert templates` subcommand in
this case. In those cases it doesn't matter whether you put the
options left or right of the enumeration argument.
