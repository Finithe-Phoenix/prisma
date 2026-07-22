#!/usr/bin/env python3
"""Apply the first compile-repair pass for PR #312.

The replacements are intentionally assertion-heavy: the script aborts rather
than silently editing an unexpected revision.
"""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def load(relative: str) -> tuple[Path, str]:
    path = ROOT / relative
    return path, path.read_text(encoding="utf-8")


def save(path: Path, text: str) -> None:
    path.write_text(text, encoding="utf-8")


def replace_exact(text: str, old: str, new: str, expected: int, label: str) -> str:
    count = text.count(old)
    if count != expected:
        raise RuntimeError(f"{label}: expected {expected} occurrence(s), found {count}")
    return text.replace(old, new)


def rename_token(text: str, old: str, new: str) -> str:
    pattern = rf"\b{re.escape(old)}\b"
    updated, count = re.subn(pattern, new, text)
    if count == 0:
        raise RuntimeError(f"token {old}: no occurrences found")
    return updated


def remove_second_line(text: str, line: str, label: str) -> str:
    needle = line + "\n"
    positions = [match.start() for match in re.finditer(re.escape(needle), text)]
    if len(positions) != 2:
        raise RuntimeError(f"{label}: expected exactly two duplicate lines, found {len(positions)}")
    second = positions[1]
    return text[:second] + text[second + len(needle) :]


# Avoid glibc's sa_handler macro while preserving the guest ABI layout.
header_path, header = load("core/include/prisma/syscall_handler.hpp")
header = replace_exact(
    header,
    "    std::uint64_t sa_handler;",
    "    std::uint64_t handler;",
    1,
    "GuestSigaction handler field",
)
save(header_path, header)

dispatcher_path, dispatcher = load("core/src/runtime/dispatcher.cpp")
dispatcher = replace_exact(
    dispatcher,
    "sa.sa_handler",
    "sa.handler",
    3,
    "dispatcher guest handler references",
)
dispatcher = replace_exact(
    dispatcher,
    "state_.gpr[static_cast<std::size_t>(ir::Gpr::Rdi)] = sig;",
    "state_.gpr[static_cast<std::size_t>(ir::Gpr::Rdi)] =\n"
    "                    static_cast<std::uint64_t>(sig);",
    1,
    "dispatcher signal argument cast",
)
save(dispatcher_path, dispatcher)

syscall_path, syscall = load("core/src/runtime/syscall_handler.cpp")

# Host headers expose these spellings as macros. Keep guest constants scoped
# and macro-proof with Prisma-style names.
for old, new in {
    "CLONE_VM": "kCloneVm",
    "CLONE_FS": "kCloneFs",
    "CLONE_FILES": "kCloneFiles",
    "CLONE_SIGHAND": "kCloneSighand",
    "CLONE_PARENT_SETTID": "kCloneParentSettid",
    "CLONE_CHILD_CLEARTID": "kCloneChildCleartid",
    "CLONE_THREAD": "kCloneThread",
    "CLONE_SETTLS": "kCloneSettls",
    "CLONE_CHILD_SETTID": "kCloneChildSettid",
    "FUTEX_WAIT": "kFutexWait",
    "FUTEX_WAKE": "kFutexWake",
    "FUTEX_PRIVATE_FLAG": "kFutexPrivateFlag",
}.items():
    syscall = rename_token(syscall, old, new)

syscall = remove_second_line(
    syscall,
    '        case kX64RtSigaction: return "rt_sigaction";',
    "duplicate rt_sigaction name",
)
syscall = remove_second_line(
    syscall,
    '        case kX64RtSigprocmask: return "rt_sigprocmask";',
    "duplicate rt_sigprocmask name",
)

syscall = replace_exact(
    syscall,
    "g_guest_sigactions[sig]",
    "g_guest_sigactions[static_cast<std::size_t>(sig)]",
    3,
    "guest sigaction indexes",
)

old_machine = (
    "                    std::uint16_t e_machine = "
    "static_cast<std::uint16_t>(header[18]) | "
    "(static_cast<std::uint16_t>(header[19]) << 8);"
)
new_machine = (
    "                    const std::uint16_t e_machine = static_cast<std::uint16_t>(\n"
    "                        static_cast<unsigned int>(header[18])\n"
    "                        | (static_cast<unsigned int>(header[19]) << 8U));"
)
syscall = replace_exact(
    syscall,
    old_machine,
    new_machine,
    1,
    "ELF machine decode",
)

syscall = replace_exact(
    syscall,
    "child_state.gpr[static_cast<std::size_t>(ir::Gpr::Rax)] = 0;",
    "child_state.gpr[static_cast<std::size_t>(Gpr::Rax)] = 0;",
    1,
    "clone child RAX",
)
syscall = replace_exact(
    syscall,
    "child_state.gpr[static_cast<std::size_t>(ir::Gpr::Rsp)] = child_stack;",
    "child_state.gpr[static_cast<std::size_t>(Gpr::Rsp)] = child_stack;",
    1,
    "clone child RSP",
)
syscall = replace_exact(
    syscall,
    "*reinterpret_cast<std::uint32_t*>(child_tidptr) = host_tid;",
    "*reinterpret_cast<std::uint32_t*>(child_tidptr) =\n"
    "                        static_cast<std::uint32_t>(host_tid);",
    1,
    "child TID write",
)
syscall = replace_exact(
    syscall,
    "*reinterpret_cast<std::uint32_t*>(parent_tidptr) = child_tid;",
    "*reinterpret_cast<std::uint32_t*>(parent_tidptr) =\n"
    "                    static_cast<std::uint32_t>(child_tid);",
    1,
    "parent TID write",
)
syscall = replace_exact(
    syscall,
    "            result = child_tid;",
    "            result = static_cast<std::int64_t>(child_tid);",
    1,
    "clone return TID cast",
)

save(syscall_path, syscall)

print("Applied PR #312 compile repair pass 1")
