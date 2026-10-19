import unittest

from case_runtime import requirement
from labels import label_key


class LabelKeyTests(unittest.TestCase):
    @requirement("REQ-1")
    def test_trim_and_collapse(self):
        self.assertEqual(label_key("  planning   board  "), "planning board")

    @requirement("REQ-1")
    def test_unicode_whitespace_and_punctuation(self):
        self.assertEqual(label_key("\u2003work\t/\nnotes\u00a0"), "work / notes")

    @requirement("REQ-2")
    def test_ascii_case(self):
        self.assertEqual(label_key("MiXeD"), "mixed")

    @requirement("REQ-2")
    def test_unicode_casefold(self):
        self.assertEqual(label_key("Straße"), "strasse")
        self.assertEqual(label_key("STRASSE"), "strasse")

    @requirement("REQ-3")
    def test_empty_fallback(self):
        self.assertEqual(label_key(""), "inbox")

    @requirement("REQ-3")
    def test_blank_fallback(self):
        self.assertEqual(label_key(" \t\u2003\n"), "inbox")
