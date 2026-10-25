//! Grounded semantic grouping: chronology, process, comparison, causal and
//! numbered structures. Split into focused submodules; every function is a
//! pure move from the former single file, with no behavior change.

mod causal;
mod chronological;
mod comparison;
mod numbered;
mod process;

pub(crate) use causal::*;
pub(crate) use chronological::*;
pub(crate) use comparison::*;
pub(crate) use numbered::*;
pub(crate) use process::*;
