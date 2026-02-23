# Hacking guide

(Be sure to read the [tooling README](../README.md) for a general
overview.)

## General

### Style / details

* Types with names ending in "Opts" (or also "Opt") are generally
  precursor types (at least if a sister type without the "Opts" suffix
  exists) used for configuration or command line options, and
  translated to another type before use. 
  
  In the case of the configuration file (deriving serde
  Serialization/Deserialization), the name is overridden without the
  "Opts" for the serialized form; e.g. the struct `RunConfigOpts` is
  really what is serialzed from/to the config file, but named
  `RunConfig` there, because that is also the type that the program
  then actually uses after converting `RunConfigOpts` to an instance
  of struct `RunConfig`.

* `Arc` is only used where unavoidable due to statically non-decidable
  life times (e.g. config file reload led to the need to make config
  values generally use it) or where shared ownership makes code
  evolution easier; i.e. as long as it's clear that a struct derives
  its contents from one particular other place, references and
  lifetime parameters on the struct are used.
  
  When cloning `Arc`, to make the fact visible that it's a cheap
  clone, `clone_arc()` from [utillib/arc.rs](../src/utillib/arc.rs) is
  generally used.


## Specifics

* [internals/eval/](internals/eval/index.md) -- how evobench-eval works internally

* [internals/jobs/](internals/jobs/index.md) -- how evobench works internally

