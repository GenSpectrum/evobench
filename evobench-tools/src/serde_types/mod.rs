//! Helper/wrapper types with good serde support
//!
//! I.e. are made explicitly for good representations that can be used
//! in config and state files. They (generally) also support
//! `FromStr`, and thus are also usable with the `clap` command line
//! parser.

pub mod allowed_env_var;
pub mod date_and_time;
pub mod git_branch_name;
pub mod git_hash;
pub mod git_reference;
pub mod git_url;
pub mod key_val;
pub mod map;
pub mod priority;
pub mod proper_dirname;
pub mod proper_filename;
pub mod regex;
pub mod tilde_path;
pub mod val_or_ref;
