# Hacking guide

Also see the [overview](overview.md).

## Style / details

* Types with names ending in "Opts" (or also "Opt" XX) are generally
  (XX?) precursor types (at least if a sister type without the "Opts"
  suffix exists): used for configuration or command line options, but
  translated before use.

* Using `Arc` for the parts that come from the config or are derived
  from it during load time, as that process is quite a bit convoluted,
  and worse, there's config file reload, too. It might still be
  feasible to use references instead, but so what. But, trying to use
  `clone_arc()` (from `src/utillib/arc.rs`) consistently whenever an
  `Arc` is cloned, for clarity and easy searching when interested
  where it happens. Please keep this up.

## State

Filesystem-based state is immediately updated on the file system to
reflect the in-memory representation, but currently not necessarily
the other way around. Filesystem-based state is:

- The set of queues (dirs created as necessary according to config)

- The jobs in the queues: bidirectionally, re-read on every job
  selection iteration. Job changes (moves between queues, failure and
  run counts, priority boost) are immediately synchronized to the file
  system.

- Working directory changes are immediately synchronized to the file
  system, but currently changes in the file system are not picked up
  (this is a todo).

The state about which is the currently executed queue (for queue
actions) is currently maintained in memory only, and also not
currently passed on re-exec of the binary--this is an open bug.

