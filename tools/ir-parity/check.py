"""Inventory IR constructors and stable C++ wire tags for issue #321."""

import argparse
import difflib
from pathlib import Path
import re
import sys


ROOT = Path(__file__).resolve().parents[2]
BASELINE = Path(__file__).with_name("operations.tsv")


def declaration(source, pattern):
    match = re.search(pattern, source, re.S)
    if not match:
        raise ValueError(f"could not locate declaration {pattern!r}")
    return match[1]


def variants(body, pattern):
    names = []
    for item in body.strip().rstrip(",").split(","):
        match = re.fullmatch(pattern, item.strip())
        if not match or match[1] in names:
            raise ValueError(f"invalid or duplicate IR variant: {item!r}")
        names.append(match[1])
    return set(names)


def cpp_source(root, path):
    source = (root / path).read_text()
    return re.sub(r"/\*.*?\*/|//[^\n]*", "", source, flags=re.S)


def wire_tags(source, enum, prefix, writer_pattern):
    body = declaration(source, rf"enum class {enum}\s*:[^{{]+\{{(.*?)\}};")
    tags = {}
    for entry in body.strip().rstrip(",").split(","):
        match = re.fullmatch(rf"\s*{prefix}(\w+)\s*=\s*(\d+)\s*", entry)
        if not match:
            raise ValueError(f"{enum}: unrecognized tag definition {entry!r}")
        name, value = match[1], int(match[2])
        if name in tags or value in tags.values():
            raise ValueError(f"{enum}: duplicate tag {name} = {value}")
        tags[name] = value
    if enum == "OpKind":
        if tags.pop("Reserved", None) != 0:
            raise ValueError("OpKind: missing reserved tag 0")

    writers = set()
    for variant, tag in re.findall(writer_pattern, source):
        if variant != tag or variant in writers:
            raise ValueError(f"{enum}: invalid writer mapping {variant} -> {tag}")
        writers.add(variant)
    readers = set(re.findall(rf"case\s+{enum}::{prefix}(\w+)\s*:", source))
    if enum == "OpKind":
        readers.discard("Reserved")
    for side, names in (("writer", writers), ("reader", readers)):
        if names != tags.keys():
            raise ValueError(f"{enum} {side} disagrees with tag table: {sorted(names ^ tags.keys())}")
    return tags


def matrix(root):
    cpp = variants(declaration(cpp_source(root, "core/include/prisma/ir.hpp"),
                               r"using Op\s*=\s*std::variant<(.*?)>;"), r"(\w+)")
    rust_source = cpp_source(root, "shell/prisma-ir/src/lib.rs")
    rust = variants(declaration(rust_source, r"pub enum Op\s*\{(.*?)\}"), r"(\w+)\(\w+\)")
    lean_src = (root / "ir-spec/PrismaIR/Syntax.lean").read_text()
    lean_body = declaration(lean_src, r"\binductive Op where\b(.*?)\bderiving\b")
    lean_body = re.sub(r"/-.*?-/|--[^\n]*", "", lean_body, flags=re.S)
    lean = re.findall(r"\|\s*(\w+)", lean_body)
    if not lean or len(lean) != len(set(lean)):
        raise ValueError("empty or duplicate Lean Op declarations")

    rfc = wire_tags(cpp_source(root, "core/src/ir/serialize.cpp"), "OpKind", "k",
                    r"is_same_v<T,\s*(\w+)>\)\s*return OpKind::k(\w+);")
    binary = wire_tags(cpp_source(root, "core/src/ir/serialization.cpp"), "OpTag", "",
                       r"is_same_v<T,\s*(\w+)>\)\s*\{\s*enum8\(OpTag::(\w+)\);")
    if cpp != rfc.keys() or not binary.keys() <= cpp:
        raise ValueError(f"C++ tag coverage mismatch: {sorted((cpp ^ rfc.keys()) | (binary.keys() - cpp))}")

    # Lean uses lower camel case, and names the C++ Return operation ret.
    spellings = {name.lower(): name for name in cpp | rust}
    lean_names = {("Return" if name == "ret" else spellings.get(name.lower(), name)): name
                  for name in lean}
    rows = ["operation\tcpp\trust\tlean_constructor\trfc0009_tag\tir_binary_tag"]
    for name in sorted(cpp | rust | lean_names.keys()):
        cells = [name, "yes" if name in cpp else "-", "yes" if name in rust else "-",
                 lean_names.get(name, "-"), str(rfc.get(name, "-")), str(binary.get(name, "-"))]
        rows.append("\t".join(cells))
    return "\n".join(rows) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, help="write the observed matrix, including on drift")
    args = parser.parse_args()
    observed = matrix(ROOT)
    if args.output:
        args.output.write_text(observed)
    baseline = BASELINE.read_text()
    if observed != baseline:
        sys.stderr.writelines(difflib.unified_diff(
            baseline.splitlines(keepends=True), observed.splitlines(keepends=True),
            fromfile="reviewed operations", tofile="observed operations"))
        return 1
    print(f"Verified {len(observed.splitlines()) - 1} operation rows")
    return 0


if __name__ == "__main__":
    sys.exit(main())
