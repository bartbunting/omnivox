#!/usr/bin/env python3
"""Generate tiny owned ONNX graphs for native model/speaker routing tests.

No trained weights, voice data, downloads or Python dependencies. Each graph
returns 128 constant samples equal to base + 0.125 * sid. Other declared Piper
inputs are intentionally unused. Field numbers follow the published schema:
https://github.com/onnx/onnx/blob/v1.17.0/onnx/onnx.proto
This is a fixture encoder, not a general protobuf or ONNX implementation.
"""
from pathlib import Path
import struct


def varint(value: int) -> bytes:
    result = bytearray()
    while value >= 128:
        result.append((value & 127) | 128)
        value >>= 7
    result.append(value)
    return bytes(result)


def integer(field: int, value: int) -> bytes:
    return varint(field << 3) + varint(value)


def blob(field: int, value: bytes | str) -> bytes:
    if isinstance(value, str):
        value = value.encode()
    return varint((field << 3) | 2) + varint(len(value)) + value


def value_info(name: str, kind: int, dimensions: list[int | str]) -> bytes:
    shape = b"".join(blob(1, integer(1, d) if isinstance(d, int) else blob(2, d)) for d in dimensions)
    tensor_type = integer(1, kind) + blob(2, shape)
    return blob(1, name) + blob(2, blob(1, tensor_type))


def tensor(name: str, dimensions: list[int], samples: list[float]) -> bytes:
    return (b"".join(integer(1, d) for d in dimensions) + integer(2, 1)
            + blob(8, name) + blob(9, struct.pack(f"<{len(samples)}f", *samples)))


def node(operation: str, inputs: list[str], output: str, attribute: bytes = b"") -> bytes:
    return (b"".join(blob(1, name) for name in inputs) + blob(2, output)
            + blob(4, operation) + (blob(5, attribute) if attribute else b""))


def model(base: float) -> bytes:
    cast_attribute = blob(1, "to") + integer(3, 1) + integer(20, 2)
    nodes = [node("Cast", ["sid"], "speaker", cast_attribute),
             node("Mul", ["speaker", "factor"], "scaled"),
             node("Add", ["scaled", "base"], "audio")]
    inputs = [("input", 7, [1, "n"]), ("input_lengths", 7, [1]),
              ("scales", 1, [3]), ("sid", 7, [1])]
    graph = (b"".join(blob(1, n) for n in nodes) + blob(2, "omnivox-speaker-fixture")
             + blob(5, tensor("factor", [1], [0.125]))
             + blob(5, tensor("base", [1, 1, 128], [base] * 128))
             + b"".join(blob(11, value_info(*spec)) for spec in inputs)
             + blob(12, value_info("audio", 1, [1, 1, 128])))
    return integer(1, 8) + blob(2, "omnivox-test-fixture") + blob(7, graph) + blob(8, integer(2, 13))


if __name__ == "__main__":
    root = Path(__file__).resolve().parent.parent / "test-fixtures/piper-speakers"
    root.mkdir(exist_ok=True)
    for name, base in [("alpha", 0.125), ("beta", 0.375)]:
        (root / f"{name}.onnx").write_bytes(model(base))
        print(f"Generated {name}.onnx ({len(model(base))} bytes)")
