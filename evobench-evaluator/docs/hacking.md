
* types with names ending in "Opts" (or also "Opt" XX) are generally (XX?) precursor types (at least if a sister type without the "Opts" suffix exists): used for configuration or command line options, but translated before use.

* Using `Arc` for the parts that come from the config or are derived
  from it during load time, as that process is quite a bit convoluted,
  and worse, there's config file reload, too. It might still be
  feasible to use references instead, but so what. But, trying to use
  `clone_arc()` (from `src/utillib/arc.rs`) consistently whenever an
  `Arc` is cloned, for clarity and easy searching when interested
  where it happens. Please keep this up.

