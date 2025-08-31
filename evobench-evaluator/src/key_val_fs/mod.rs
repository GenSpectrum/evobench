//! Simple filesystem based key-value database using a separate file
//! in the file system per mapping, and offering locking operations on
//! each entry for mutations/deletions. The goal of this library is
//! not speed, but reliability, locking features, and ease of
//! inspection of the state with standard command line tools.

pub mod as_key;
pub mod key_val;
pub mod queue;
