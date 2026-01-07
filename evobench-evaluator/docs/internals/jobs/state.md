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

