import unittest

from check import variants, wire_tags


class DeclarationTests(unittest.TestCase):
    def test_new_variant_shape_requires_review(self):
        with self.assertRaisesRegex(ValueError, "NewOp"):
            variants("Load(Load), NewOp { reg: Ref },", r"(\w+)\(\w+\)")

    def test_both_wire_visitors_must_cover_every_tag(self):
        declaration = "enum class OpTag : u8 { A=1, B=2 };"
        readers = "case OpTag::A: case OpTag::B:"
        writers = "write(A, A); write(B, B);"
        pattern = r"write\((\w+), (\w+)\)"
        self.assertEqual(wire_tags(declaration + readers + writers, "OpTag", "", pattern),
                         {"A": 1, "B": 2})
        for missing, expected in (("case OpTag::A:", "reader"), ("write(B, B)", "writer")):
            with self.subTest(missing=missing), self.assertRaisesRegex(ValueError, expected):
                wire_tags((declaration + readers + writers).replace(missing, ""), "OpTag", "", pattern)
        with self.assertRaisesRegex(ValueError, "invalid writer mapping"):
            wire_tags(declaration + readers + writers.replace("B, B", "B, A"), "OpTag", "", pattern)
        with self.assertRaisesRegex(ValueError, "duplicate tag"):
            wire_tags(declaration.replace("B=2", "B=1") + readers + writers, "OpTag", "", pattern)
