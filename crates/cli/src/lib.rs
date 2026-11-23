//! The internals of the `rngo` command-line tool, published only so the CLI's own tests can call
//! them. This is not a stable API: it may change in any release, including a patch release. To run
//! simulations from Rust, use the [`rngo`](https://docs.rs/rngo) crate.

#[doc(hidden)]
pub mod eval;
#[doc(hidden)]
pub mod init;
#[doc(hidden)]
pub mod skills;
mod ui;
