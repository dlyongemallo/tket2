//! Quantum circuit representation and operations.

pub mod command;
pub mod cost;
mod hash;
pub mod units;

use std::iter::Sum;

pub use command::{Command, CommandIterator};
pub use hash::CircuitHash;
use itertools::Either::{Left, Right};

use derive_more::From;
use hugr::hugr::hugrmut::HugrMut;
use hugr::hugr::NodeType;
use hugr::ops::dataflow::IOTrait;
use hugr::ops::{Input, NamedOp, OpParent, OpTag, OpTrait, Output, DFG};
use hugr::types::PolyFuncType;
use hugr::{Hugr, PortIndex};
use hugr::{HugrView, OutgoingPort};
use itertools::Itertools;
use thiserror::Error;

pub use hugr::ops::OpType;
pub use hugr::types::{EdgeKind, Type, TypeRow};
pub use hugr::{Node, Port, Wire};

use self::units::{filter, LinearUnit, Units};

/// A quantum circuit, represented as a function in a HUGR.
#[derive(Debug, Clone, PartialEq)]
pub struct Circuit<T = Hugr> {
    /// The HUGR containing the circuit.
    hugr: T,
    /// The parent node of the circuit.
    ///
    /// This is checked at runtime to ensure that the node is a DFG node.
    parent: Node,
}

impl<T: Default + HugrView> Default for Circuit<T> {
    fn default() -> Self {
        let hugr = T::default();
        let parent = hugr.root();
        Self { hugr, parent }
    }
}

impl<T: HugrView> Circuit<T> {
    /// Create a new circuit from a HUGR and a node.
    ///
    /// # Errors
    ///
    /// Returns an error if the parent node is not a DFG node in the HUGR.
    pub fn try_new(hugr: T, parent: Node) -> Result<Self, CircuitError> {
        check_hugr(&hugr, parent)?;
        Ok(Self { hugr, parent })
    }

    /// Create a new circuit from a HUGR and a node.
    ///
    /// See [`Circuit::try_new`] for a version that returns an error.
    ///
    /// # Panics
    ///
    /// Panics if the parent node is not a DFG node in the HUGR.
    pub fn new(hugr: T, parent: Node) -> Self {
        Self::try_new(hugr, parent).unwrap_or_else(|e| panic!("{}", e))
    }

    /// Returns the node containing the circuit definition.
    pub fn parent(&self) -> Node {
        self.parent
    }

    /// Get a reference to the HUGR containing the circuit.
    pub fn hugr(&self) -> &T {
        &self.hugr
    }

    /// Unwrap the HUGR containing the circuit.
    pub fn into_hugr(self) -> T {
        self.hugr
    }

    /// Get a mutable reference to the HUGR containing the circuit.
    ///
    /// Mutation of the hugr MUST NOT invalidate the parent node,
    /// by changing the node's type to a non-DFG node or by removing it.
    pub fn hugr_mut(&mut self) -> &mut T {
        &mut self.hugr
    }

    /// Ensures the circuit contains an owned HUGR.
    pub fn to_owned(&self) -> Circuit<Hugr> {
        let hugr = self.hugr.base_hugr().clone();
        Circuit {
            hugr,
            parent: self.parent,
        }
    }

    /// Return the name of the circuit
    #[inline]
    pub fn name(&self) -> Option<&str> {
        self.hugr.get_metadata(self.parent(), "name")?.as_str()
    }

    /// Returns the function type of the circuit.
    ///
    /// Equivalent to [`HugrView::get_function_type`].
    #[inline]
    pub fn circuit_signature(&self) -> PolyFuncType {
        let op = self.hugr.get_optype(self.parent);
        match op {
            OpType::FuncDecl(decl) => decl.signature.clone(),
            OpType::FuncDefn(defn) => defn.signature.clone(),
            _ => op
                .inner_function_type()
                .expect("Circuit parent should have a function type")
                .into(),
        }
    }

    /// Returns the input node to the circuit.
    #[inline]
    pub fn input_node(&self) -> Node {
        self.hugr
            .get_io(self.parent)
            .expect("Circuit has no input node")[0]
    }

    /// Returns the output node to the circuit.
    #[inline]
    pub fn output_node(&self) -> Node {
        self.hugr
            .get_io(self.parent)
            .expect("Circuit has no output node")[1]
    }

    /// Returns the input and output nodes of the circuit.
    #[inline]
    pub fn io_nodes(&self) -> [Node; 2] {
        self.hugr
            .get_io(self.parent)
            .expect("Circuit has no I/O nodes")
    }

    /// The number of quantum gates in the circuit.
    #[inline]
    pub fn num_gates(&self) -> usize
    where
        Self: Sized,
    {
        // TODO: Discern quantum gates in the commands iterator.
        self.hugr().children(self.parent).count() - 2
    }

    /// Count the number of qubits in the circuit.
    #[inline]
    pub fn qubit_count(&self) -> usize
    where
        Self: Sized,
    {
        self.qubits().count()
    }

    /// Get the input units of the circuit and their types.
    #[inline]
    pub fn units(&self) -> Units<OutgoingPort>
    where
        Self: Sized,
    {
        Units::new_circ_input(self)
    }

    /// Get the linear input units of the circuit and their types.
    #[inline]
    pub fn linear_units(&self) -> impl Iterator<Item = (LinearUnit, OutgoingPort, Type)> + '_
    where
        Self: Sized,
    {
        self.units().filter_map(filter::filter_linear)
    }

    /// Get the non-linear input units of the circuit and their types.
    #[inline]
    pub fn nonlinear_units(&self) -> impl Iterator<Item = (Wire, OutgoingPort, Type)> + '_
    where
        Self: Sized,
    {
        self.units().filter_map(filter::filter_non_linear)
    }

    /// Returns the units corresponding to qubits inputs to the circuit.
    #[inline]
    pub fn qubits(&self) -> impl Iterator<Item = (LinearUnit, OutgoingPort, Type)> + '_
    where
        Self: Sized,
    {
        self.units().filter_map(filter::filter_qubit)
    }

    /// Returns all the commands in the circuit, in some topological order.
    ///
    /// Ignores the Input and Output nodes.
    #[inline]
    pub fn commands(&self) -> CommandIterator<'_, T>
    where
        Self: Sized,
    {
        // Traverse the circuit in topological order.
        CommandIterator::new(self)
    }

    /// Compute the cost of the circuit based on a per-operation cost function.
    #[inline]
    pub fn circuit_cost<F, C>(&self, op_cost: F) -> C
    where
        Self: Sized,
        C: Sum,
        F: Fn(&OpType) -> C,
    {
        self.commands().map(|cmd| op_cost(cmd.optype())).sum()
    }

    /// Compute the cost of a group of nodes in a circuit based on a
    /// per-operation cost function.
    #[inline]
    pub fn nodes_cost<F, C>(&self, nodes: impl IntoIterator<Item = Node>, op_cost: F) -> C
    where
        C: Sum,
        F: Fn(&OpType) -> C,
    {
        nodes
            .into_iter()
            .map(|n| op_cost(self.hugr.get_optype(n)))
            .sum()
    }

    /// Return the graphviz representation of the underlying graph and hierarchy side by side.
    ///
    /// For a simpler representation, use the [`Circuit::mermaid_string`] format instead.
    pub fn dot_string(&self) -> String {
        // TODO: This will print the whole HUGR without identifying the circuit container.
        // Should we add some extra formatting for that?
        self.hugr.dot_string()
    }

    /// Return the mermaid representation of the underlying hierarchical graph.
    ///
    /// The hierarchy is represented using subgraphs. Edges are labelled with
    /// their source and target ports.
    ///
    /// For a more detailed representation, use the [`Circuit::dot_string`]
    /// format instead.
    pub fn mermaid_string(&self) -> String {
        // TODO: See comment in `dot_string`.
        self.hugr.mermaid_string()
    }
}

impl<T: HugrView> From<T> for Circuit<T> {
    fn from(hugr: T) -> Self {
        let parent = hugr.root();
        Self::new(hugr, parent)
    }
}

/// Checks if the passed hugr is a valid circuit,
/// and return [`CircuitError`] if not.
fn check_hugr(hugr: &impl HugrView, parent: Node) -> Result<(), CircuitError> {
    if !hugr.contains_node(parent) {
        return Err(CircuitError::MissingParentNode { parent });
    }
    let optype = hugr.get_optype(parent);
    if !OpTag::DataflowParent.is_superset(optype.tag()) {
        return Err(CircuitError::NonDFGParent {
            parent,
            optype: optype.clone(),
        });
    }
    Ok(())
}

/// Remove an empty wire in a dataflow HUGR.
///
/// The wire to be removed is identified by the index of the outgoing port
/// at the circuit input node.
///
/// This will change the circuit signature and will shift all ports after
/// the removed wire by -1. If the wire is connected to the output node,
/// this will also change the signature output and shift the ports after
/// the removed wire by -1.
///
/// This will return an error if the wire is not empty or if a HugrError
/// occurs.
#[allow(dead_code)]
pub(crate) fn remove_empty_wire(
    circ: &mut Circuit<impl HugrMut>,
    input_port: usize,
) -> Result<(), CircuitMutError> {
    let parent = circ.parent();
    let hugr = circ.hugr_mut();

    let [inp, out] = hugr.get_io(parent).expect("no IO nodes found at parent");
    if input_port >= hugr.num_outputs(inp) {
        return Err(CircuitMutError::InvalidPortOffset(input_port));
    }
    let input_port = OutgoingPort::from(input_port);
    let link = hugr
        .linked_inputs(inp, input_port)
        .at_most_one()
        .map_err(|_| CircuitMutError::DeleteNonEmptyWire(input_port.index()))?;
    if link.is_some() && link.unwrap().0 != out {
        return Err(CircuitMutError::DeleteNonEmptyWire(input_port.index()));
    }
    if link.is_some() {
        hugr.disconnect(inp, input_port);
    }

    // Shift ports at input
    shift_ports(hugr, inp, input_port, hugr.num_outputs(inp))?;
    // Shift ports at output
    if let Some((out, output_port)) = link {
        shift_ports(hugr, out, output_port, hugr.num_inputs(out))?;
    }
    // Update input node, output node (if necessary) and parent signatures.
    update_signature(
        hugr,
        parent,
        input_port.index(),
        link.map(|(_, p)| p.index()),
    );
    // Resize ports at input/output node
    hugr.set_num_ports(inp, 0, hugr.num_outputs(inp) - 1);
    if let Some((out, _)) = link {
        hugr.set_num_ports(out, hugr.num_inputs(out) - 1, 0);
    }
    Ok(())
}

/// Errors that can occur when mutating a circuit.
#[derive(Debug, Clone, Error, PartialEq)]
pub enum CircuitError {
    /// The parent node for the circuit does not exist in the HUGR.
    #[error("{parent} cannot define a circuit as it is not present in the HUGR.")]
    MissingParentNode {
        /// The node that was used as the parent.
        parent: Node,
    },
    /// The parent node for the circuit is not a DFG node.
    #[error(
        "{parent} cannot be used as a circuit parent. A {} is not a dataflow container.",
        optype.name()
    )]
    NonDFGParent {
        /// The node that was used as the parent.
        parent: Node,
        /// The parent optype.
        optype: OpType,
    },
}

/// Errors that can occur when mutating a circuit.
#[derive(Debug, Clone, Error, PartialEq, Eq, From)]
pub enum CircuitMutError {
    /// A Hugr error occurred.
    #[error("Hugr error: {0:?}")]
    HugrError(hugr::hugr::HugrError),
    /// The wire to be deleted is not empty.
    #[from(ignore)]
    #[error("Wire {0} cannot be deleted: not empty")]
    DeleteNonEmptyWire(usize),
    /// The wire does not exist.
    #[from(ignore)]
    #[error("Wire {0} does not exist")]
    InvalidPortOffset(usize),
}

/// Shift ports in range (free_port + 1 .. max_ind) by -1.
fn shift_ports<C: HugrMut + ?Sized>(
    circ: &mut C,
    node: Node,
    free_port: impl Into<Port>,
    max_ind: usize,
) -> Result<Port, hugr::hugr::HugrError> {
    let mut free_port = free_port.into();
    let dir = free_port.direction();
    let port_range = (free_port.index() + 1..max_ind).map(|p| Port::new(dir, p));
    for port in port_range {
        let links = circ.linked_ports(node, port).collect_vec();
        if !links.is_empty() {
            circ.disconnect(node, port);
        }
        for (other_n, other_p) in links {
            match other_p.as_directed() {
                Right(other_p) => {
                    let dst_port = free_port.as_incoming().unwrap();
                    circ.connect(other_n, other_p, node, dst_port)
                }
                Left(other_p) => {
                    let src_port = free_port.as_outgoing().unwrap();
                    circ.connect(node, src_port, other_n, other_p)
                }
            };
        }
        free_port = port;
    }
    Ok(free_port)
}

// Update the signature of circ when removing the in_index-th input wire and
// the out_index-th output wire.
fn update_signature(
    hugr: &mut impl HugrMut,
    parent: Node,
    in_index: usize,
    out_index: Option<usize>,
) {
    let inp = hugr
        .get_io(parent)
        .expect("no IO nodes found at circuit parent")[0];
    // Update input node
    let inp_types: TypeRow = {
        let OpType::Input(Input { types }) = hugr.get_optype(inp).clone() else {
            panic!("invalid circuit")
        };
        let mut types = types.into_owned();
        types.remove(in_index);
        types.into()
    };
    let new_inp_op = Input::new(inp_types.clone());
    let inp_exts = hugr.get_nodetype(inp).input_extensions().cloned();
    hugr.replace_op(inp, NodeType::new(new_inp_op, inp_exts))
        .unwrap();

    // Update output node if necessary.
    let out_types = out_index.map(|out_index| {
        let out = hugr.get_io(parent).unwrap()[1];
        let out_types: TypeRow = {
            let OpType::Output(Output { types }) = hugr.get_optype(out).clone() else {
                panic!("invalid circuit")
            };
            let mut types = types.into_owned();
            types.remove(out_index);
            types.into()
        };
        let new_out_op = Output::new(out_types.clone());
        let inp_exts = hugr.get_nodetype(out).input_extensions().cloned();
        hugr.replace_op(out, NodeType::new(new_out_op, inp_exts))
            .unwrap();
        out_types
    });

    // Update parent
    let OpType::DFG(DFG { mut signature, .. }) = hugr.get_optype(parent).clone() else {
        panic!("invalid circuit")
    };
    signature.input = inp_types;
    if let Some(out_types) = out_types {
        signature.output = out_types;
    }
    let new_dfg_op = DFG { signature };
    let inp_exts = hugr.get_nodetype(parent).input_extensions().cloned();
    hugr.replace_op(parent, NodeType::new(new_dfg_op, inp_exts))
        .unwrap();
}

#[cfg(test)]
mod tests {
    use cool_asserts::assert_matches;

    use hugr::types::FunctionType;
    use hugr::{
        builder::{DFGBuilder, DataflowHugr},
        extension::{prelude::BOOL_T, PRELUDE_REGISTRY},
    };

    use super::*;
    use crate::{json::load_tk1_json_str, utils::build_simple_circuit, Tk2Op};

    fn test_circuit() -> Circuit {
        load_tk1_json_str(
            r#"{ "phase": "0",
            "bits": [["c", [0]]],
            "qubits": [["q", [0]], ["q", [1]]],
            "commands": [
                {"args": [["q", [0]]], "op": {"type": "H"}},
                {"args": [["q", [0]], ["q", [1]]], "op": {"type": "CX"}},
                {"args": [["q", [1]]], "op": {"type": "X"}}
            ],
            "implicit_permutation": [[["q", [0]], ["q", [0]]], [["q", [1]], ["q", [1]]]]
        }"#,
        )
        .unwrap()
    }

    #[test]
    fn test_circuit_properties() {
        let circ = test_circuit();

        assert_eq!(circ.name(), None);
        assert_eq!(circ.circuit_signature().body().input_count(), 3);
        assert_eq!(circ.circuit_signature().body().output_count(), 3);
        assert_eq!(circ.qubit_count(), 2);
        assert_eq!(circ.num_gates(), 3);

        assert_eq!(circ.units().count(), 3);
        assert_eq!(circ.nonlinear_units().count(), 0);
        assert_eq!(circ.linear_units().count(), 3);
        assert_eq!(circ.qubits().count(), 2);
    }

    #[test]
    fn remove_qubit() {
        let mut circ = build_simple_circuit(2, |circ| {
            circ.append(Tk2Op::X, [0])?;
            Ok(())
        })
        .unwrap();

        assert_eq!(circ.qubit_count(), 2);
        assert!(remove_empty_wire(&mut circ, 1).is_ok());
        assert_eq!(circ.qubit_count(), 1);
        assert_eq!(
            remove_empty_wire(&mut circ, 0).unwrap_err(),
            CircuitMutError::DeleteNonEmptyWire(0)
        );
    }

    #[test]
    fn test_invalid_parent() {
        let hugr = Hugr::default();

        assert_matches!(
            Circuit::try_new(hugr.clone(), hugr.root()),
            Err(CircuitError::NonDFGParent { .. }),
        );
    }

    #[test]
    fn remove_bit() {
        let h = DFGBuilder::new(FunctionType::new(vec![BOOL_T], vec![])).unwrap();
        let mut circ: Circuit = h
            .finish_hugr_with_outputs([], &PRELUDE_REGISTRY)
            .unwrap()
            .into();

        assert_eq!(circ.units().count(), 1);
        assert!(remove_empty_wire(&mut circ, 0).is_ok());
        assert_eq!(circ.units().count(), 0);
        assert_eq!(
            remove_empty_wire(&mut circ, 2).unwrap_err(),
            CircuitMutError::InvalidPortOffset(2)
        );
    }
}
