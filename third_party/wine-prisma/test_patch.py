from __future__ import annotations

import subprocess
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
PATCH = ROOT / "third_party" / "wine-prisma" / "patches" / (
    "0001-prisma-no-preload-reserve.patch"
)
WINE = ROOT / "third_party" / "wine"
DOCKERFILE = ROOT / "docker" / "Dockerfile.wine-arm64"


class WineArm64EcPatchTests(unittest.TestCase):
    def test_patch_applies_to_pinned_wine(self) -> None:
        result = subprocess.run(
            ["git", "-C", str(WINE), "apply", "--check", str(PATCH)],
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_initial_x64_entry_uses_hybrid_dispatch(self) -> None:
        patch = PATCH.read_text(encoding="utf-8")
        expected = (
            '+         "mov x11, x0\\n\\t"       /* entry */',
            '+         "adr x10, $iexit_thunk$cdecl$i8$i8\\n\\t"',
            '+         "mov x0, x1\\n\\t"        /* arg */',
            '          "adrp x16, __os_arm64x_dispatch_icall\\n\\t"',
            '          "ldr x16, [x16, #:lo12:__os_arm64x_dispatch_icall]\\n\\t"',
            '          "blr x16\\n\\t"',
            '          "blr x11\\n\\t"',
            '+         "bl \\"#RtlExitUserThread\\"\\n\\t"',
        )
        positions = [patch.index(line) for line in expected]
        self.assertEqual(positions, sorted(positions))

    def test_runtime_build_audits_compiled_thread_thunk(self) -> None:
        dockerfile = DOCKERFILE.read_text(encoding="utf-8")
        self.assertIn(
            "llvm-objdump --disassemble-symbols=RtlUserThreadStart",
            dockerfile,
        )
        self.assertIn("<$iexit_thunk$cdecl$i8$i8>", dockerfile)
        self.assertIn("<RtlExitUserThread>", dockerfile)
        self.assertIn("! grep -Fq '<$iexit_thunk$cdecl$v$i8i8i8>'", dockerfile)


if __name__ == "__main__":
    unittest.main()
