//! Bindings for rust-defined operations

use derive_more::{From, Into};
use hugr::hugr::IdentList;
use hugr::ops::custom::{ExtensionOp, OpaqueOp};
use hugr::types::FunctionType;
use pyo3::prelude::*;
use std::fmt;
use std::str::FromStr;
use strum::IntoEnumIterator;

use hugr::ops::{CustomOp, NamedOp, OpType};
use tket2::{Pauli, Tk2Op};

use crate::types::PyHugrType;
use crate::utils::into_vec;

/// The module definition
pub fn module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    let m = PyModule::new_bound(py, "ops")?;
    m.add_class::<PyTk2Op>()?;
    m.add_class::<PyPauli>()?;
    m.add_class::<PyCustomOp>()?;
    Ok(m)
}

/// Enum of Tket2 operations in hugr.
///
/// Python equivalent of [`Tk2Op`].
///
/// [`Tk2Op`]: tket2::ops::Tk2Op
#[pyclass]
#[pyo3(name = "Tk2Op")]
#[derive(Debug, Clone, From)]
#[repr(transparent)]
pub struct PyTk2Op {
    /// Rust representation of the operation.
    pub op: Tk2Op,
}

#[pymethods]
impl PyTk2Op {
    /// Initialize a new `PyTk2Op` from a python string.
    #[new]
    fn new(op: &str) -> PyResult<Self> {
        Ok(Self {
            op: Tk2Op::from_str(op)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?,
        })
    }

    /// Iterate over the operations.
    #[staticmethod]
    pub fn values() -> PyTk2OpIter {
        PyTk2OpIter { it: Tk2Op::iter() }
    }

    /// Get the string name of the operation.
    #[getter]
    pub fn name(&self) -> String {
        self.op.name().to_string()
    }

    /// Get the qualified name of the operation, including the extension.
    #[getter]
    pub fn qualified_name(&self) -> String {
        self.op.exposed_name().to_string()
    }

    /// Wrap the operation as a custom operation.
    pub fn to_custom(&self) -> PyCustomOp {
        let custom: ExtensionOp = self.op.into_extension_op();
        CustomOp::new_extension(custom).into()
    }

    /// String representation of the operation.
    pub fn __repr__(&self) -> String {
        self.qualified_name()
    }

    /// String representation of the operation.
    pub fn __str__(&self) -> String {
        self.name()
    }

    /// Check if two operations are equal.
    pub fn __eq__(&self, other: &PyTk2Op) -> bool {
        self.op == other.op
    }
}

/// Iterator over the operations.
#[pyclass]
#[pyo3(name = "Tk2OpIter")]
#[derive(Debug, Clone)]
#[repr(transparent)]
pub struct PyTk2OpIter {
    /// Iterator over the operations.
    pub it: <Tk2Op as IntoEnumIterator>::Iterator,
}

#[pymethods]
impl PyTk2OpIter {
    /// Iterate over the operations.
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Get the next operation.
    pub fn __next__(&mut self) -> Option<PyTk2Op> {
        self.it.next().map(|op| PyTk2Op { op })
    }
}

/// Pauli matrices
///
/// Python equivalent of [`Pauli`].
///
/// [`Pauli`]: tket2::ops::Pauli
#[pyclass]
#[pyo3(name = "Pauli")]
#[derive(Debug, Clone, From)]
#[repr(transparent)]
pub struct PyPauli {
    /// Rust representation of the pauli matrix.
    pub p: Pauli,
}

#[pymethods]
impl PyPauli {
    /// Initialize a new `PyPauli` from a python string.
    #[new]
    fn new(p: &str) -> PyResult<Self> {
        Ok(Self {
            p: Pauli::from_str(p)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?,
        })
    }

    /// Iterate over the pauli matrices.
    #[staticmethod]
    pub fn values() -> PyPauliIter {
        PyPauliIter { it: Pauli::iter() }
    }

    /// Return the Pauli matrix as a string.
    #[getter]
    pub fn name(&self) -> String {
        format!("{}", self.p)
    }

    /// Pauli identity matrix.
    #[staticmethod]
    #[pyo3(name = "I")]
    fn i() -> Self {
        Self { p: Pauli::I }
    }

    /// Pauli X matrix.
    #[staticmethod]
    #[pyo3(name = "X")]
    fn x() -> Self {
        Self { p: Pauli::X }
    }

    /// Pauli Y matrix.
    #[staticmethod]
    #[pyo3(name = "Y")]
    fn y() -> Self {
        Self { p: Pauli::Y }
    }

    /// Pauli Z matrix.
    #[pyo3(name = "Z")]
    #[staticmethod]
    fn z() -> Self {
        Self { p: Pauli::Z }
    }

    /// String representation of the Pauli matrix.
    pub fn __repr__(&self) -> String {
        format!("{:?}", self.p)
    }

    /// String representation of the Pauli matrix.
    pub fn __str__(&self) -> String {
        format!("{}", self.p)
    }

    /// Check if two Pauli matrices are equal.
    pub fn __eq__(&self, other: &Bound<PyAny>) -> bool {
        let Ok(other): Result<&Bound<PyPauli>, _> = other.downcast() else {
            return false;
        };
        self.p == other.borrow().p
    }
}

/// Iterator over the Pauli matrices.
#[pyclass]
#[pyo3(name = "PauliIter")]
#[derive(Debug, Clone)]
#[repr(transparent)]
pub struct PyPauliIter {
    /// Iterator over the Pauli matrices.
    pub it: <Pauli as IntoEnumIterator>::Iterator,
}

#[pymethods]
impl PyPauliIter {
    /// Iterate over the operations.
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Get the next Pauli matrix.
    pub fn __next__(&mut self) -> Option<PyPauli> {
        self.it.next().map(|p| PyPauli { p })
    }
}

/// A wrapped custom operation.
#[pyclass]
#[pyo3(name = "CustomOp")]
#[repr(transparent)]
#[derive(From, Into, PartialEq, Clone)]
pub struct PyCustomOp(CustomOp);

impl fmt::Debug for PyCustomOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<PyCustomOp> for OpType {
    fn from(op: PyCustomOp) -> Self {
        op.0.into()
    }
}

#[pymethods]
impl PyCustomOp {
    #[new]
    fn new(
        extension: &str,
        op_name: &str,
        input_types: Vec<PyHugrType>,
        output_types: Vec<PyHugrType>,
    ) -> PyResult<Self> {
        Ok(CustomOp::new_opaque(OpaqueOp::new(
            IdentList::new(extension).unwrap(),
            op_name,
            Default::default(),
            [],
            FunctionType::new(into_vec(input_types), into_vec(output_types)),
        ))
        .into())
    }

    fn to_custom(&self) -> Self {
        self.clone()
    }

    /// String representation of the operation.
    pub fn __repr__(&self) -> String {
        format!("{:?}", self)
    }

    #[getter]
    fn name(&self) -> String {
        self.0.name().to_string()
    }
}
