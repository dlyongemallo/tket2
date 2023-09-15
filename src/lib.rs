#![warn(missing_docs)]

//! TKET2: The Hardware Agnostic Quantum Compiler
//!
//! TKET2 is an open source quantum compiler developed by Quantinuum. Central to
//! TKET2's design is its hardware agnosticism which allows researchers and
//! quantum software developers to take advantage of its state of the art
//! compilation for many different quantum architectures.

pub mod circuit;
pub mod extension;
pub mod json;
pub(crate) mod ops;
pub mod optimiser;
pub mod rewrite;
pub use ops::{symbolic_constant_op, Pauli, T2Op};

#[cfg(feature = "portmatching")]
pub mod portmatching;

mod utils;
