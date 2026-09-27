#!/usr/bin/env python3
"""Regression checks for broken links missed by the former Markdown-only gate."""

from pathlib import Path
import tempfile
import unittest

import check_documentation_links as check


class DocumentationLinks(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.repositories = {"omnivox": self.root}

    def write(self, name, text):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        return path

    def errors(self, name, text, **options):
        return check.check_document(self.write(name, text), self.root, self.repositories, **options)

    def test_renamed_file_and_fragment_fail_independently(self):
        self.write("guides/current.md", "# Current\n## Use `voice`!\n")
        errors = self.errors("README.md", "[old](old.md)\n[bad](guides/current.md#absent)\n[good](guides/current.md#use-voice)\n")
        self.assertEqual(len(errors), 2)
        self.assertIn("missing target", errors[0])
        self.assertIn("missing anchor", errors[1])

    def test_duplicate_headings_and_fenced_examples(self):
        text = "# Guide\n## Topic\n## Topic\n```md\n## Fake\n[bad](missing.md)\n```\n[one](#topic) [two](#topic-1) [bad](#fake)\n"
        errors = self.errors("guide.md", text)
        self.assertEqual(len(errors), 1)
        self.assertIn("#fake", errors[0])

    def test_org_file_heading_custom_id_and_missing_target(self):
        self.write("format.org", "* Layout\n:PROPERTIES:\n:CUSTOM_ID: stable\n:END:\n")
        errors = self.errors("guide.org", "[[file:format.org::*Layout][section]]\n[[file:format.org::#stable]]\n[[file:missing.org]]\n[[file:format.org::*Missing]]\n")
        self.assertEqual(len(errors), 2)

    def test_org_examples_are_not_checked_as_prose(self):
        self.assertEqual(self.errors("guide.org", "#+begin_src org\n[[file:missing.org]]\n#+end_src\n* Real\n[[Real]]\n"), [])

    def test_reference_links_escaped_paths_and_explicit_anchors(self):
        self.write("a file.md", '<a id="explicit"></a>\nTitle\n=====\n')
        self.assertEqual(self.errors("README.md", '[file](<a file.md#explicit>)\n[encoded](a%20file.md#title)\n[label]: a%20file.md#title\n'), [])

    def test_current_github_links_checked_but_historical_urls_preserved(self):
        base = "https://github.com/bartbunting/omnivox/blob/"
        self.write("docs/new.md", "# New\n")
        errors = self.errors("README.md", f"[ok]({base}main/docs/new.md#new)\n[old]({base}main/OLD.md)\n[historical]({base}ca38781/OLD.md)\n[unrelated](https://example.com/absent)\n")
        self.assertEqual(len(errors), 1)
        self.assertIn("OLD.md", errors[0])

    def test_incoming_scan_only_checks_links_to_omnivox(self):
        errors = self.errors("peer.org", "[[file:unrelated.org]]\n[[https://github.com/bartbunting/omnivox/blob/main/OLD.md]]\n", incoming_only=True)
        self.assertEqual(len(errors), 1)

    def test_cross_repository_outgoing_link_uses_selected_checkout(self):
        peer = self.root / "peer"; peer.mkdir()
        (peer / "README.org").write_text("* Start\n", encoding="utf-8")
        self.repositories["emacsvox"] = peer
        self.assertEqual(self.errors("README.md", "[peer](https://github.com/bartbunting/emacsvox/blob/master/README.org#start)\n"), [])

    def test_repository_escape_and_invalid_line_range_rejected(self):
        self.write("source.rs", "one\ntwo\n")
        errors = self.errors("README.md", "[escape](../outside)\n[line](source.rs#L2-L1)\n[good](source.rs#L1-L2)\n")
        self.assertEqual(len(errors), 2)


if __name__ == "__main__":
    unittest.main()
