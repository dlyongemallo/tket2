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
mod ops;
pub mod passes;
pub use ops::{Pauli, T2Op};

#[cfg(feature = "portmatching")]
pub mod portmatching;

mod utils;
