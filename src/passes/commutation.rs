use std::{collections::HashMap, rc::Rc};

use hugr::{
    hugr::{hugrmut::HugrMut, CircuitUnit, HugrError, PortIndex, Rewrite},
    Direction, Hugr, HugrView, Node, Port,
};
use itertools::Itertools;
use portgraph::PortOffset;

#[cfg(feature = "pyo3")]
use pyo3::{create_exception, exceptions::PyException, PyErr};

use crate::{
    circuit::{command::Command, units::filter::Qubits, Circuit},
    ops::{Pauli, T2Op},
};

use thiserror::Error;

type Qb = crate::circuit::units::LinearUnit;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
// remove once https://github.com/CQCL-DEV/tket2/issues/126 is resolved
struct ComCommand {
    /// The operation node.
    node: Node,
    /// An assignment of linear units to the node's ports.
    //
    // We'll need something more complex if `follow_linear_port` stops being a
    // direct map from input to output.
    inputs: Vec<CircuitUnit>,
}

impl<'c, Circ> From<Command<'c, Circ>> for ComCommand
where
    Circ: HugrView,
{
    fn from(com: Command<'c, Circ>) -> Self {
        ComCommand {
            node: com.node(),
            inputs: com.inputs().map(|(c, _, _)| c).collect(),
        }
    }
}
impl ComCommand {
    fn node(&self) -> Node {
        self.node
    }
    fn qubits(&self) -> impl Iterator<Item = Qb> + '_ {
        self.inputs.iter().filter_map(|u| {
            let CircuitUnit::Linear(i) = u else {
                return None;
            };
            Some(Qb::new(*i))
        })
    }
    fn port_of_qb(&self, qb: Qb, direction: Direction) -> Option<Port> {
        self.inputs
            .iter()
            .position(|cu| {
                let q_cu: CircuitUnit = qb.into();
                cu == &q_cu
            })
            .map(|i| PortOffset::new(direction, i).into())
    }
}

type Slice = Vec<Option<Rc<ComCommand>>>;
type SliceVec = Vec<Slice>;

fn add_to_slice(slice: &mut Slice, com: Rc<ComCommand>) {
    for q in com.qubits() {
        slice[q.index()] = Some(com.clone());
    }
}

fn load_slices(circ: &impl Circuit) -> SliceVec {
    let mut slices = vec![];

    let n_qbs = circ.units().filter_units::<Qubits>().count();
    let mut qubit_free_slice = vec![0; n_qbs];

    for command in circ.commands().filter(|c| is_slice_op(circ, c.node())) {
        let command: ComCommand = command.into();
        let free_slice = command
            .qubits()
            .map(|qb| qubit_free_slice[qb.index()])
            .max()
            .unwrap();

        for q in command.qubits() {
            qubit_free_slice[q.index()] = free_slice + 1;
        }
        if free_slice >= slices.len() {
            debug_assert!(free_slice == slices.len());
            slices.push(vec![None; n_qbs]);
        }
        let command = Rc::new(command);
        add_to_slice(&mut slices[free_slice], command);
    }

    slices
}

/// check if node is one we want to put in to a slice.
fn is_slice_op(h: &impl HugrView, node: Node) -> bool {
    let op: Result<T2Op, _> = h.get_optype(node).clone().try_into();
    op.is_ok()
}

/// Starting from starting_index, work back along slices to check for the
/// earliest slice that can accommodate this command, if any.
fn available_slice(
    circ: &impl HugrView,
    slice_vec: &[Slice],
    starting_index: usize,
    command: &Rc<ComCommand>,
) -> Option<(usize, HashMap<Qb, Rc<ComCommand>>)> {
    let mut available = None;
    let mut prev_nodes: HashMap<Qb, Rc<ComCommand>> = HashMap::new();

    for slice_index in (0..starting_index + 1).rev() {
        // if all qubit slots are empty here the command can be moved here
        if command
            .qubits()
            .all(|q| slice_vec[slice_index][q.index()].is_none())
        {
            available = Some((slice_index, prev_nodes.clone()));
        } else if slice_index == 0 {
            break;
        } else {
            // if command commutes with all ports here it can be moved past,
            // otherwise stop
            if let Some(new_prev_nodes) = commutes_at_slice(command, &slice_vec[slice_index], circ)
            {
                prev_nodes.extend(new_prev_nodes);
            } else {
                break;
            }
        }
    }

    available
}

// If a command commutes back through this slice return a map from the qubits of
// the command to the commands in this slice acting on those qubits.
fn commutes_at_slice(
    command: &Rc<ComCommand>,
    slice: &Slice,
    circ: &impl HugrView,
) -> Option<HashMap<Qb, Rc<ComCommand>>> {
    // map from qubit to node it is connected to immediately after the free slice.
    let mut prev_nodes: HashMap<Qb, Rc<ComCommand>> =
        HashMap::from_iter(command.qubits().map(|q| (q, command.clone())));

    for q in command.qubits() {
        // if slot is empty, continue checking.
        let Some(other_com) = &slice[q.index()] else {
            continue;
        };

        let port = command.port_of_qb(q, Direction::Incoming)?;

        let op: T2Op = circ.get_optype(command.node()).clone().try_into().ok()?;
        // TODO: if not t2op, might still have serialized commutation data we
        // can use.
        let pauli = commutation_on_port(&op.qubit_commutation(), port)?;

        let other_op: T2Op = circ.get_optype(other_com.node()).clone().try_into().ok()?;
        let other_pauli = commutation_on_port(
            &other_op.qubit_commutation(),
            other_com.port_of_qb(q, Direction::Outgoing)?,
        )?;

        if pauli.commutes_with(other_pauli) {
            prev_nodes.insert(q, other_com.clone());
        } else {
            return None;
        }
    }

    Some(prev_nodes)
}

fn commutation_on_port(comms: &[(usize, Pauli)], port: Port) -> Option<Pauli> {
    comms
        .iter()
        .find_map(|(i, p)| (*i == port.index()).then_some(*p))
}

/// Error from a [`PullForward`] operation.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum PullForwardError {
    // Error in hugr mutation.
    #[error("Hugr mutation error: {0:?}")]
    HugrError(#[from] HugrError),

    #[error("Qubit {0} not found in command.")]
    NoQbInCommand(usize),

    #[error("No subsequent command found for qubit {0}")]
    NoCommandForQb(usize),
}

#[cfg(feature = "pyo3")]
create_exception!(
    pyrs,
    PyPullForwardError,
    PyException,
    "Error in applying PullForward rewrite."
);

#[cfg(feature = "pyo3")]
impl From<PullForwardError> for PyErr {
    fn from(err: PullForwardError) -> Self {
        PyPullForwardError::new_err(err.to_string())
    }
}
struct PullForward {
    command: Rc<ComCommand>,
    new_nexts: HashMap<Qb, Rc<ComCommand>>,
}

impl Rewrite for PullForward {
    type Error = PullForwardError;

    type ApplyResult = ();

    const UNCHANGED_ON_FAILURE: bool = false;

    fn verify(&self, _h: &impl HugrView) -> Result<(), Self::Error> {
        unimplemented!()
    }

    fn apply(self, h: &mut impl HugrMut) -> Result<Self::ApplyResult, Self::Error> {
        let Self { command, new_nexts } = self;

        let qb_port = |command: &ComCommand, qb, direction| {
            command
                .port_of_qb(qb, direction)
                .ok_or(PullForwardError::NoQbInCommand(qb.index()))
        };
        // for each qubit, disconnect node and reconnect at destination.
        for qb in command.qubits() {
            let out_port = qb_port(&command, qb, Direction::Outgoing)?;
            let in_port = qb_port(&command, qb, Direction::Incoming)?;

            let (src, src_port) = h
                .linked_ports(command.node(), in_port)
                .exactly_one()
                .ok() // PortLinks don't implement Debug
                .unwrap();
            let (dst, dst_port) = h
                .linked_ports(command.node(), out_port)
                .exactly_one()
                .ok()
                .unwrap();

            let Some(new_neighbour_com) = new_nexts.get(&qb) else {
                return Err(PullForwardError::NoCommandForQb(qb.index()));
            };
            if new_neighbour_com == &command {
                // do not need to commute along this qubit.
                continue;
            }
            h.disconnect(command.node(), in_port)?;
            h.disconnect(command.node(), out_port)?;
            // connect old source and destination - identity operation.
            h.connect(src, src_port.index(), dst, dst_port.index())?;

            let new_dst_port = qb_port(new_neighbour_com, qb, Direction::Incoming)?;
            let (new_src, new_src_port) = h
                .linked_ports(new_neighbour_com.node(), new_dst_port)
                .exactly_one()
                .ok()
                .unwrap();
            // disconnect link which we will insert in to.
            h.disconnect(new_neighbour_com.node(), new_dst_port)?;

            h.connect(
                new_src,
                new_src_port.index(),
                command.node(),
                in_port.index(),
            )?;
            h.connect(
                command.node(),
                out_port.index(),
                new_neighbour_com.node(),
                new_dst_port.index(),
            )?;
        }
        Ok(())
    }
}

/// Pass which greedily commutes operations forwards in order to reduce depth.
pub fn apply_greedy_commutation(circ: &mut Hugr) -> Result<u32, PullForwardError> {
    let mut count = 0;
    let mut slice_vec = load_slices(circ);

    for slice_index in 0..slice_vec.len() {
        let slice_commands: Vec<_> = slice_vec[slice_index]
            .iter()
            .flatten()
            .unique()
            .cloned()
            .collect();

        for command in slice_commands {
            if let Some((destination, new_nexts)) =
                available_slice(&circ, &slice_vec, slice_index, &command)
            {
                debug_assert!(
                    destination < slice_index,
                    "Avoid mutating slices we haven't got to yet."
                );
                for q in command.qubits() {
                    let com = slice_vec[slice_index][q.index()].take();
                    slice_vec[destination][q.index()] = com;
                }
                let rewrite = PullForward { command, new_nexts };
                circ.apply_rewrite(rewrite)?;
                count += 1;
            }
        }
    }

    // TODO remove empty slices and return
    // and full slices at start?
    Ok(count)
}

#[cfg(test)]
mod test {

    use crate::{extension::REGISTRY, ops::test::t2_bell_circuit, utils::build_simple_circuit};
    use hugr::{
        builder::{DFGBuilder, Dataflow, DataflowHugr},
        extension::prelude::{BOOL_T, QB_T},
        std_extensions::arithmetic::float_types::FLOAT64_TYPE,
        type_row,
        types::FunctionType,
        Hugr,
    };
    use itertools::Itertools;
    use rstest::{fixture, rstest};

    use super::*;

    #[fixture]
    // example circuit from original task
    fn example_cx() -> Hugr {
        build_simple_circuit(4, |circ| {
            circ.append(T2Op::CX, [0, 2])?;
            circ.append(T2Op::CX, [1, 2])?;
            circ.append(T2Op::CX, [1, 3])?;
            Ok(())
        })
        .unwrap()
    }

    #[fixture]
    // example circuit from original task with lower depth
    fn example_cx_better() -> Hugr {
        build_simple_circuit(4, |circ| {
            circ.append(T2Op::CX, [0, 2])?;
            circ.append(T2Op::CX, [1, 3])?;
            circ.append(T2Op::CX, [1, 2])?;
            Ok(())
        })
        .unwrap()
    }

    #[fixture]
    // can't commute anything here
    fn cant_commute() -> Hugr {
        build_simple_circuit(4, |circ| {
            circ.append(T2Op::Z, [1])?;
            circ.append(T2Op::CX, [0, 1])?;
            circ.append(T2Op::CX, [2, 1])?;
            Ok(())
        })
        .unwrap()
    }

    #[fixture]
    fn big_example() -> Hugr {
        build_simple_circuit(4, |circ| {
            circ.append(T2Op::CX, [0, 3])?;
            circ.append(T2Op::CX, [1, 2])?;
            circ.append(T2Op::H, [0])?;
            circ.append(T2Op::H, [3])?;
            circ.append(T2Op::CX, [0, 1])?;
            circ.append(T2Op::CX, [2, 3])?;
            circ.append(T2Op::CX, [0, 1])?;
            circ.append(T2Op::CX, [2, 3])?;
            circ.append(T2Op::CX, [2, 1])?;
            circ.append(T2Op::H, [1])?;
            Ok(())
        })
        .unwrap()
    }

    #[fixture]
    // commute a single qubit gate
    fn single_qb_commute() -> Hugr {
        build_simple_circuit(3, |circ| {
            circ.append(T2Op::H, [1])?;
            circ.append(T2Op::CX, [0, 1])?;
            circ.append(T2Op::Z, [0])?;
            Ok(())
        })
        .unwrap()
    }
    #[fixture]

    // commute 2 single qubit gates
    fn single_qb_commute_2() -> Hugr {
        build_simple_circuit(4, |circ| {
            circ.append(T2Op::CX, [1, 2])?;
            circ.append(T2Op::CX, [1, 0])?;
            circ.append(T2Op::CX, [3, 2])?;
            circ.append(T2Op::X, [0])?;
            circ.append(T2Op::Z, [3])?;
            Ok(())
        })
        .unwrap()
    }

    #[fixture]
    // A commutation forward exists but depth doesn't change
    fn commutes_but_same_depth() -> Hugr {
        build_simple_circuit(3, |circ| {
            circ.append(T2Op::H, [1])?;
            circ.append(T2Op::CX, [0, 1])?;
            circ.append(T2Op::Z, [0])?;
            circ.append(T2Op::X, [1])?;
            Ok(())
        })
        .unwrap()
    }

    #[fixture]
    // Gate being commuted has a non-linear input
    fn non_linear_inputs() -> Hugr {
        let build = || {
            let mut dfg = DFGBuilder::new(FunctionType::new(
                type_row![QB_T, QB_T, FLOAT64_TYPE],
                type_row![QB_T, QB_T],
            ))?;

            let [q0, q1, f] = dfg.input_wires_arr();

            let mut circ = dfg.as_circuit(vec![q0, q1]);

            circ.append(T2Op::H, [1])?;
            circ.append(T2Op::CX, [0, 1])?;
            circ.append_and_consume(T2Op::RzF64, [CircuitUnit::Linear(0), CircuitUnit::Wire(f)])?;
            let qbs = circ.finish();
            dfg.finish_hugr_with_outputs(qbs, &REGISTRY)
        };
        build().unwrap()
    }

    #[fixture]
    // Gates being commuted have non-linear outputs
    fn non_linear_outputs() -> Hugr {
        let build = || {
            let mut dfg = DFGBuilder::new(FunctionType::new(
                type_row![QB_T, QB_T],
                type_row![QB_T, QB_T, BOOL_T],
            ))?;

            let [q0, q1] = dfg.input_wires_arr();

            let mut circ = dfg.as_circuit(vec![q0, q1]);

            circ.append(T2Op::H, [1])?;
            circ.append(T2Op::CX, [0, 1])?;
            let measured = circ.append_with_outputs(T2Op::Measure, [0])?;
            let mut outs = circ.finish();
            outs.extend(measured);
            dfg.finish_hugr_with_outputs(outs, &REGISTRY)
        };
        build().unwrap()
    }

    fn slice_from_command(
        commands: &[ComCommand],
        n_qbs: usize,
        slice_arr: &[&[usize]],
    ) -> SliceVec {
        slice_arr
            .iter()
            .map(|command_indices| {
                let mut slice = vec![None; n_qbs];
                for ind in command_indices.iter() {
                    let com = commands[*ind].clone();
                    add_to_slice(&mut slice, Rc::new(com))
                }

                slice
            })
            .collect()
    }

    #[rstest]
    fn test_load_slices_cx(example_cx: Hugr) {
        let circ = example_cx;
        let commands: Vec<ComCommand> = circ.commands().map_into().collect();
        let slices = load_slices(&circ);
        let correct = slice_from_command(&commands, 4, &[&[0], &[1], &[2]]);

        assert_eq!(slices, correct);
    }

    #[rstest]
    fn test_load_slices_cx_better(example_cx_better: Hugr) {
        let circ = example_cx_better;
        let commands: Vec<ComCommand> = circ.commands().map_into().collect();

        let slices = load_slices(&circ);
        let correct = slice_from_command(&commands, 4, &[&[0, 1], &[2]]);

        assert_eq!(slices, correct);
    }

    #[rstest]
    fn test_load_slices_bell(t2_bell_circuit: Hugr) {
        let circ = t2_bell_circuit;
        let commands: Vec<ComCommand> = circ.commands().map_into().collect();

        let slices = load_slices(&circ);
        let correct = slice_from_command(&commands, 2, &[&[0], &[1]]);

        assert_eq!(slices, correct);
    }

    #[rstest]
    fn test_available_slice(example_cx: Hugr) {
        let circ = example_cx;
        let slices = load_slices(&circ);
        let (found, prev_nodes) =
            available_slice(&circ, &slices, 1, slices[2][1].as_ref().unwrap()).unwrap();
        assert_eq!(found, 0);

        assert_eq!(
            *prev_nodes.get(&Qb::new(1)).unwrap(),
            slices[1][1].as_ref().unwrap().clone()
        );

        assert_eq!(
            *prev_nodes.get(&Qb::new(3)).unwrap(),
            slices[2][3].as_ref().unwrap().clone()
        );
    }

    #[rstest]
    fn big_test(big_example: Hugr) {
        let circ = big_example;
        let slices = load_slices(&circ);
        assert_eq!(slices.len(), 6);
        // can commute final cx to front
        let (found, prev_nodes) =
            available_slice(&circ, &slices, 3, slices[4][1].as_ref().unwrap()).unwrap();
        assert_eq!(found, 1);
        assert_eq!(
            *prev_nodes.get(&Qb::new(1)).unwrap(),
            slices[2][1].as_ref().unwrap().clone()
        );

        assert_eq!(
            *prev_nodes.get(&Qb::new(2)).unwrap(),
            slices[2][2].as_ref().unwrap().clone()
        );
        // hadamard can't commute past anything
        assert!(available_slice(&circ, &slices, 4, slices[5][1].as_ref().unwrap()).is_none());
    }

    /// Calculate depth by placing commands in slices.
    fn depth(h: &Hugr) -> usize {
        load_slices(h).len()
    }
    #[rstest]
    #[case(example_cx(), true, 1)]
    #[case(example_cx_better(), false, 0)]
    #[case(big_example(), true, 1)]
    #[case(cant_commute(), false, 0)]
    #[case(t2_bell_circuit(), false, 0)]
    #[case(single_qb_commute(), true, 1)]
    #[case(single_qb_commute_2(), true, 2)]
    #[case(commutes_but_same_depth(), false, 1)]
    #[case(non_linear_inputs(), true, 1)]
    #[case(non_linear_outputs(), true, 1)]
    fn commutation_example(
        #[case] mut case: Hugr,
        #[case] should_reduce: bool,
        #[case] expected_moves: u32,
    ) {
        let node_count = case.node_count();
        let depth_before = depth(&case);
        let move_count = apply_greedy_commutation(&mut case).unwrap();
        case.infer_and_validate(&REGISTRY).unwrap();

        assert_eq!(
            move_count, expected_moves,
            "Number of commutations did not match expected."
        );
        let depth_after = depth(&case);

        if should_reduce {
            assert!(depth_after < depth_before, "Depth should have decreased..");
        } else {
            assert_eq!(
                depth_before, depth_after,
                "Depth should not have changed for this case."
            );
        }

        assert_eq!(
            case.node_count(),
            node_count,
            "depth optimisation should not change the number of nodes."
        )
    }
}
