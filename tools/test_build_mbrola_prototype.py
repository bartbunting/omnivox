#!/usr/bin/env python3
"""Check complete, minimal regeneration of the private en1 frontend data."""
from pathlib import Path
import tempfile
import unittest

from build_mbrola_prototype import FRONTEND_DATA, stage_frontend_data


class StagingTests(unittest.TestCase):
    def test_rebuild_removes_unused_languages_and_preserves_english_data(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, target = root / "source", root / "target"
            for name in (*FRONTEND_DATA, "fr_dict", "voices/mb/mb-fr1"):
                path = source / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(name.encode())
            target.mkdir()
            (target / "fr_dict").write_bytes(b"stale language")
            stage_frontend_data(source, target)
            self.assertFalse((target / "fr_dict").exists())
            self.assertFalse((target / "voices/mb/mb-fr1").exists())
            self.assertEqual((target / "en_dict").read_bytes(), b"en_dict")
            self.assertEqual((target / "voices/mb/mb-en1").read_bytes(), b"voices/mb/mb-en1")
            (source / "phondata").unlink()
            with self.assertRaises(FileNotFoundError):
                stage_frontend_data(source, target)
            self.assertEqual((target / "en_dict").read_bytes(), b"en_dict")


if __name__ == "__main__":
    unittest.main()
