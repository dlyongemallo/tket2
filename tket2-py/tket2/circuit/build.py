from typing import Protocol, Iterable
from tket2.circuit import HugrType, CustomOp, Dfg, Node, Wire, Tk2Circuit
from dataclasses import dataclass

QB_T = HugrType.qubit()
LB_T = HugrType.linear_bit()
BOOL_T = HugrType.bool()


class Command(Protocol):
    """Interface to specify a custom operation over some qubits and linear bits.
    Refers to qubits and bits by index."""

    gate_name: str
    n_qb: int
    n_lb: int = 0
    extension_name: str = "quantum.tket2"

    def qubits(self) -> list[int]: ...
    def bits(self) -> list[int]:
        return []

    @classmethod
    def op(cls) -> CustomOp:
        types = [QB_T] * cls.n_qb + [LB_T] * cls.n_lb
        return CustomOp(cls.extension_name, cls.gate_name, types, types)


class CircBuild:
    """Helper class to build a circuit from commands by tracking qubits,
    allowing commands to be specified by qubit index."""

    dfg: Dfg
    qbs: list[Wire]

    def __init__(self, n_qb: int) -> None:
        self.dfg = Dfg([QB_T] * n_qb, [QB_T] * n_qb)
        self.qbs = self.dfg.inputs()

    def add(self, op: CustomOp, indices: list[int]) -> Node:
        """Add a Custom operation to some qubits and update the qubit list."""
        qbs = [self.qbs[i] for i in indices]
        n = self.dfg.add_op(op, qbs)
        outs = n.outs(len(indices))
        for i, o in zip(indices, outs):
            self.qbs[i] = o

        return n

    def measure_all(self) -> list[Wire]:
        """Append a measurement to all qubits and return the measurement result wires."""
        return [self.add(Measure, [i]).outs(2)[1] for i in range(len(self.qbs))]

    def add_command(self, command: Command) -> Node:
        """Add a Command to the circuit and return the new node."""
        return self.add(command.op(), command.qubits())

    def extend(self, coms: Iterable[Command]) -> "CircBuild":
        """Add a sequence of commands to the circuit."""
        for op in coms:
            self.add_command(op)
        return self

    def finish(self) -> Tk2Circuit:
        """Finish building the circuit by setting all the qubits as the output
        and validate."""
        return self.dfg.finish(self.qbs)


def from_coms(*args: Command) -> Tk2Circuit:
    """Build a circuit from a sequence of commands, assuming only qubit outputs."""
    commands = []
    n_qb = 0
    # traverses commands twice which isn't great
    for arg in args:
        max_qb = max(arg.qubits()) + 1
        n_qb = max(n_qb, max_qb)
        commands.append(arg)

    build = CircBuild(n_qb)
    build.extend(commands)
    return build.finish()


# Some common operations

# Define some "Commands" for pure quantum gates (n qubits in and n qubits out)


@dataclass(frozen=True)
class H(Command):
    qubit: int
    gate_name = "H"
    n_qb = 1

    def qubits(self) -> list[int]:
        return [self.qubit]


@dataclass(frozen=True)
class CX(Command):
    control: int
    target: int
    gate_name = "CX"
    n_qb = 2

    def qubits(self) -> list[int]:
        return [self.control, self.target]


@dataclass(frozen=True)
class PauliX(Command):
    qubit: int
    gate_name = "X"
    n_qb = 1

    def qubits(self) -> list[int]:
        return [self.qubit]


@dataclass(frozen=True)
class PauliZ(Command):
    qubit: int
    gate_name = "Z"
    n_qb = 1

    def qubits(self) -> list[int]:
        return [self.qubit]


@dataclass(frozen=True)
class PauliY(Command):
    qubit: int
    gate_name = "Y"
    n_qb = 1

    def qubits(self) -> list[int]:
        return [self.qubit]


# Define CustomOps for common operations that don't have an (n qubits in, n
# qubits out) signature
QAlloc = CustomOp("quantum.tket2", "QAlloc", [], [QB_T])
QFree = CustomOp("quantum.tket2", "QFree", [QB_T], [])
Measure = CustomOp("quantum.tket2", "Measure", [QB_T], [QB_T, BOOL_T])
Not = CustomOp("logic", "Not", [BOOL_T], [BOOL_T])
