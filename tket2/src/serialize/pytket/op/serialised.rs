//! Wrapper over pytket operations that cannot be represented naturally in tket2.

use hugr::extension::prelude::QB_T;

use hugr::ops::custom::{CustomOp, ExtensionOp};
use hugr::ops::{NamedOp, OpType};
use hugr::std_extensions::arithmetic::float_types::FLOAT64_TYPE;
use hugr::types::type_param::CustomTypeArg;
use hugr::types::{FunctionType, TypeArg};

use hugr::IncomingPort;
use serde::de::Error;
use tket_json_rs::circuit_json;

use crate::extension::{
    LINEAR_BIT, REGISTRY, TKET1_EXTENSION, TKET1_EXTENSION_ID, TKET1_OP_NAME, TKET1_OP_PAYLOAD,
};
use crate::serialize::pytket::OpConvertError;

/// A serialized operation, containing the operation type and all its attributes.
///
/// This value is only used if the operation does not have a native TKET2
/// counterpart that can be represented as a [`NativeOp`].
///
/// Wrapper around [`tket_json_rs::circuit_json::Operation`] with cached number of qubits and bits.
///
/// The `Operation` contained by this struct is guaranteed to have a signature.
///
///   [`NativeOp`]: super::native::NativeOp
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OpaqueTk1Op {
    /// Internal operation data.
    op: circuit_json::Operation,
    /// Number of qubits declared by the operation.
    num_qubits: usize,
    /// Number of bits declared by the operation.
    num_bits: usize,
    /// Node input for each parameter in `op.params`.
    ///
    /// If the input is `None`, the parameter does not use a Hugr port and is
    /// instead stored purely as metadata for the `Operation`.
    param_inputs: Vec<Option<IncomingPort>>,
    /// The number of non-None inputs in `param_inputs`, corresponding to the
    /// FLOAT64_TYPE inputs to the Hugr operation.
    num_params: usize,
}

impl OpaqueTk1Op {
    /// Create a new `OpaqueTk1Op` from a `circuit_json::Operation`, with the number
    /// of qubits and bits explicitly specified.
    ///
    /// If the operation does not define a signature, one is generated with the
    /// given amounts.
    pub fn new_from_op(
        mut op: circuit_json::Operation,
        num_qubits: usize,
        num_bits: usize,
    ) -> Self {
        if op.signature.is_none() {
            op.signature =
                Some([vec!["Q".into(); num_qubits], vec!["B".into(); num_bits]].concat());
        }
        let mut op = Self {
            op,
            num_qubits,
            num_bits,
            param_inputs: Vec::new(),
            num_params: 0,
        };
        op.compute_param_fields();
        op
    }

    /// Try to convert a tket2 operation into a `OpaqueTk1Op`.
    ///
    /// Only succeeds if the operation is a [`CustomOp`] containing a tket1 operation
    /// from the [`TKET1_EXTENSION_ID`] extension. Returns `None` if the operation
    /// is not a tket1 operation.
    ///
    /// # Errors
    ///
    /// Returns an [`OpConvertError`] if the operation is a tket1 operation, but it
    /// contains invalid data.
    pub fn try_from_tket2(op: &OpType) -> Result<Option<Self>, OpConvertError> {
        let OpType::CustomOp(custom_op) = op else {
            return Ok(None);
        };

        // TODO: Check `extensions.contains(&TKET1_EXTENSION_ID)`
        // (but the ext op extensions are an empty set?)
        if custom_op.name() != format!("{TKET1_EXTENSION_ID}.{TKET1_OP_NAME}") {
            return Ok(None);
        }
        let Some(TypeArg::Opaque { arg }) = custom_op.args().first() else {
            return Err(serde_yaml::Error::custom(
                "Opaque TKET1 operation did not have a yaml-encoded type argument.",
            )
            .into());
        };
        let op = serde_yaml::from_value(arg.value.clone())?;
        Ok(Some(op))
    }

    /// Compute the signature of the operation.
    ///
    /// The signature returned has `num_qubits` qubit inputs, followed by
    /// `num_bits` bit inputs, followed by `num_params` `f64` inputs. It has
    /// `num_qubits` qubit outputs followed by `num_bits` bit outputs.
    #[inline]
    pub fn signature(&self) -> FunctionType {
        let linear = [
            vec![QB_T; self.num_qubits],
            vec![LINEAR_BIT.clone(); self.num_bits],
        ]
        .concat();
        let params = vec![FLOAT64_TYPE; self.num_params];
        FunctionType::new([linear.clone(), params].concat(), linear)
            .with_extension_delta(TKET1_EXTENSION_ID)
    }

    /// Returns the ports corresponding to parameters for this operation.
    pub fn param_ports(&self) -> impl Iterator<Item = IncomingPort> + '_ {
        self.param_inputs.iter().filter_map(|&i| i)
    }

    /// Returns the lower level `circuit_json::Operation` contained by this struct.
    pub fn serialised_op(&self) -> &circuit_json::Operation {
        &self.op
    }

    /// Wraps the op into a [`TKET1_OP_NAME`] opaque operation.
    pub fn as_custom_op(&self) -> CustomOp {
        let op = serde_yaml::to_value(self).unwrap();
        let payload = TypeArg::Opaque {
            arg: CustomTypeArg::new(TKET1_OP_PAYLOAD.clone(), op).unwrap(),
        };
        let op_def = TKET1_EXTENSION.get_op(&TKET1_OP_NAME).unwrap();
        ExtensionOp::new(op_def.clone(), vec![payload], &REGISTRY)
            .unwrap_or_else(|e| panic!("{e}"))
            .into()
    }

    /// Compute the `parameter_input` and `num_params` fields by looking for
    /// parameters in `op.params` that can be mapped to input wires in the Hugr.
    ///
    /// Updates the internal `num_params` and `param_inputs` fields.
    fn compute_param_fields(&mut self) {
        let Some(params) = self.op.params.as_ref() else {
            self.param_inputs = vec![];
            self.num_params = 0;
            return;
        };

        self.num_params = params.len();
        self.param_inputs = (0..params.len()).map(|i| Some(i.into())).collect();
    }
}
