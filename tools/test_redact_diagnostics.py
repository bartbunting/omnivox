#!/usr/bin/env python3
"""Regression tests for diagnostic text redaction."""

from __future__ import annotations

import unittest

import redact_diagnostics


class DiagnosticRedactionTests(unittest.TestCase):
    def test_removes_synthesis_text_records_without_removing_lifecycle(self) -> None:
        text = (
            "INFO lifecycle_stage=worker_started dispatch_id=9\n"
            'INFO synthesis_text="private words" Captured synthesis text\n'
            "INFO lifecycle_stage=worker_finished dispatch_id=9\n"
        )

        redacted = redact_diagnostics.redact(text, [])

        self.assertNotIn("private words", redacted)
        self.assertEqual(redacted.count("lifecycle_stage="), 2)

    def test_redacts_explicit_and_common_user_home_paths(self) -> None:
        text = "\n".join(
            (
                "/srv/private/omnivox/target/release/omnivox",
                "/home/alice/src/omnivox",
                r"C:\Users\Alice\src\omnivox\omnivox.exe",
                r"\\wsl.localhost\Ubuntu\home\alice\src\omnivox",
            )
        )

        redacted = redact_diagnostics.redact(text, ["/srv/private/omnivox"])

        self.assertNotIn("alice", redacted.casefold())
        self.assertNotIn("/srv/private/omnivox", redacted)
        self.assertIn("<PRIVATE_PATH>/target/release/omnivox", redacted)
        self.assertEqual(redacted.count("<USER_HOME>"), 3)

    def test_does_not_treat_root_as_a_private_replacement(self) -> None:
        self.assertEqual(
            redact_diagnostics.redact("/usr/bin/omnivox\n", ["/", ""]),
            "/usr/bin/omnivox\n",
        )


if __name__ == "__main__":
    unittest.main()
