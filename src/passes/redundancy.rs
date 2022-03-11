use std::collections::HashSet;

use crate::circuit::circuit::Circuit;
use crate::circuit::dag::{Edge, EdgeProperties, Vertex, VertexProperties, DAG};
use crate::circuit::operation::{Op, Param};
use crate::graph::graph::{Direction, EdgeIndex};
use crate::graph::substitute::Cut;

fn get_boundary(dag: &DAG, node: Vertex, direction: Direction) -> Vec<Edge> {
    dag.node_edges(node, direction).cloned().collect()
}
fn get_weights(dag: &DAG, edges: &Vec<EdgeIndex>) -> Vec<EdgeProperties> {
    edges
        .iter()
        .map(|e| dag.edge_weight(*e).unwrap().clone())
        .collect()
}

fn add_neighbours(dag: &DAG, preds: &Vec<Edge>, succs: &Vec<Edge>, set: &mut HashSet<Vertex>) {
    for (it, i) in [(preds.iter(), 0), (succs.iter(), 1)] {
        for e in it {
            set.insert(dag.edge_endpoints(*e).unwrap()[i].node);
        }
    }
}

// A version of the redundancy removal in TKET but with only identity and dagger removal
pub fn remove_redundancies(mut circ: Circuit) -> Circuit {
    let dag = &mut circ.dag;
    let mut candidate_nodes: HashSet<_> = dag.nodes().collect();

    while !candidate_nodes.is_empty() {
        let candidate = candidate_nodes
            .take(&candidate_nodes.iter().next().cloned().unwrap())
            .unwrap();
        let op = match dag.node_weight(candidate) {
            None => continue,
            Some(VertexProperties { op }) => match op {
                Op::Input | Op::Output => continue,
                _ => op,
            },
        };

        if let Some(phase) = op.identity_up_to_phase() {
            let preds: Vec<_> = get_boundary(dag, candidate, Direction::Incoming);
            let succs: Vec<_> = get_boundary(dag, candidate, Direction::Outgoing);

            add_neighbours(dag, &preds, &succs, &mut candidate_nodes);

            circ.phase = circ.phase + Param::from(phase);
            let new_weights = get_weights(dag, &preds);
            dag.replace_with_identity(Cut::new(preds, succs), new_weights);
            continue;
        }

        let kids: HashSet<_> = dag
            .node_edges(candidate, Direction::Outgoing)
            .filter_map(|e| {
                let [start, end] = dag.edge_endpoints(*e).unwrap();
                if start.port == end.port {
                    Some(end.node)
                } else {
                    None
                }
            })
            .collect();

        if kids.len() != 1 {
            continue;
        }

        let kid = *kids.iter().next().unwrap();

        if let Some(dagged) = dag.node_weight(kid).unwrap().op.dagger() {
            if op != &dagged {
                continue;
            }

            let preds: Vec<_> = get_boundary(dag, candidate, Direction::Incoming);
            let succs: Vec<_> = get_boundary(dag, kid, Direction::Outgoing);
            let new_weights = get_weights(dag, &preds);

            add_neighbours(dag, &preds, &succs, &mut candidate_nodes);
            dag.replace_with_identity(Cut::new(preds, succs), new_weights);
            continue;
        }
    }

    circ
}
