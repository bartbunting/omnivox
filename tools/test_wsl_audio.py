#!/usr/bin/env python3
"""WSLg trial isolation, launch, measurement, and process-cleanup regression tests."""

from __future__ import annotations

import argparse
from contextlib import redirect_stdout
import io
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import wsl_audio


FAKE_LAUNCHER = """#!PYTHON
import json, os, sys
if os.environ.get("EMACSVOX_OMNIVOX_PROBE_ONLY") == "1":
    print(os.environ.get("OMNIVOX_PROGRAM") or os.environ["FAKE_WINDOWS_PROGRAM"])
elif sys.argv[1:] == ["--version"]:
    print("omnivox 1.8.0")
elif os.environ.get("FAKE_IDLE"):
    sys.stdin.read()
else:
    keys = ("TTS_PROGRAM", "OMNIVOX_PROGRAM", "OMNIVOX_AUDIO_OUTPUT",
            "OMNIVOX_ENGINE", "PULSE_LATENCY_MSEC", "OMNIVOX_RHVOICE_LIBRARY",
            "OMNIVOX_LOG_DIRECTORY", "ALSA_CONFIG_PATH")
    print(json.dumps({"args": sys.argv[1:],
                      "env": {key: os.environ.get(key) for key in keys}}))
"""


@unittest.skipUnless(os.name == "posix", "WSL trial launchers require POSIX")
class WslAudioTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="wsl-audio-'quoted ")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.emacsvox = self.root / "emacsvox"
        for relative in ("servers/omnivox", "bin/emacsvox"):
            path = self.emacsvox / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(FAKE_LAUNCHER.replace("PYTHON", sys.executable))
            path.chmod(0o700)
        self.linux = self.root / "payload/omnivox"
        self.linux.parent.mkdir()
        self.linux.write_text("#!/bin/sh\nexit 0\n")
        self.linux.chmod(0o700)
        self.windows = self.linux.with_name("omnivox.exe")
        self.windows.write_text("#!/bin/sh\nexit 0\n")
        self.windows.chmod(0o700)
        for name in ("espeak-ng-data", "third-party-licenses"):
            (self.linux.parent / name).mkdir()
        self.plugins = self.root / "plugins"
        self.plugins.mkdir()
        for kind in ("pcm", "ctl"):
            (self.plugins / f"libasound_module_{kind}_pulse.so").touch()
        self.pulse = socket.socket(socket.AF_UNIX)
        self.addCleanup(self.pulse.close)
        self.pulse_path = self.root / "pulse"
        self.pulse.bind(str(self.pulse_path))
        self.directory = self.root / "trial $(touch unintended)"
        self.args = argparse.Namespace(
            directory=self.directory, emacsvox_dir=self.emacsvox,
            linux_program=self.linux, windows_program=None,
            alsa_plugin_dir=self.plugins, pulse_server=f"unix:{self.pulse_path}",
            latency_ms="50",
        )
        self.env_patch = patch.dict(os.environ, {
            "FAKE_WINDOWS_PROGRAM": str(self.windows),
            "OMNIVOX_WSL_LATENCY_MS": "50",
        })
        self.env_patch.start()
        self.addCleanup(self.env_patch.stop)
        with redirect_stdout(io.StringIO()):
            wsl_audio.prepare(self.args)
        self.config = wsl_audio.read_config(self.directory)

    def run_launcher(self, target: str, *arguments: str, **env: str) -> dict:
        result = subprocess.run(
            [str(self.directory / f"emacsvox-{target}-test"), *arguments],
            cwd=self.root,
            env={**os.environ, **env}, stdin=subprocess.DEVNULL,
            capture_output=True, text=True, timeout=10, check=True,
        )
        return json.loads(result.stdout)

    def test_linux_launcher_preserves_arguments_and_isolates_platform_paths(self) -> None:
        result = self.run_launcher(
            "linux", "-nw", "--eval", '(message "hello $world")',
            OMNIVOX_PROGRAM=str(self.windows), TTS_PROGRAM="outloud",
            OMNIVOX_AUDIO_OUTPUT="null", OMNIVOX_RHVOICE_LIBRARY=r"C:\Voice\RHVoice.dll",
        )
        self.assertEqual(result["args"], ["-nw", "--eval", '(message "hello $world")'])
        self.assertEqual(result["env"]["OMNIVOX_PROGRAM"], str(self.linux))
        self.assertEqual(result["env"]["TTS_PROGRAM"], "omnivox")
        self.assertEqual(result["env"]["OMNIVOX_AUDIO_OUTPUT"], "device")
        self.assertEqual(result["env"]["PULSE_LATENCY_MSEC"], "50")
        self.assertIsNone(result["env"]["OMNIVOX_RHVOICE_LIBRARY"])
        self.assertFalse((self.root / "unintended").exists())

    def test_native_pulse_skips_alsa_and_does_not_change_windows_selection(self) -> None:
        inherited = {"OMNIVOX_WSL_AUDIO_OUTPUT": "pulse", "PULSE_LATENCY_MSEC": "50",
                     "ALSA_CONFIG_PATH": "old", "ALSA_PLUGIN_DIR": "old"}
        env, _ = wsl_audio.environment(self.config, self.directory, "linux", inherited)
        self.assertEqual(env["OMNIVOX_AUDIO_OUTPUT"], "pulse")
        self.assertEqual(env["OMNIVOX_PULSE_LATENCY_MS"], "40")
        tuned, _ = wsl_audio.environment(self.config, self.directory, "linux",
            {**inherited, "OMNIVOX_PULSE_LATENCY_MS": "20"})
        self.assertEqual(tuned["OMNIVOX_PULSE_LATENCY_MS"], "20")
        for name in ("ALSA_CONFIG_PATH", "ALSA_PLUGIN_DIR", "PULSE_LATENCY_MSEC"):
            self.assertNotIn(name, env)
        env, _ = wsl_audio.environment(self.config, self.directory, "windows", inherited)
        self.assertEqual(env["OMNIVOX_AUDIO_OUTPUT"], "device")

    def test_native_pulse_launcher_needs_no_alsa_plugin(self) -> None:
        for path in self.plugins.iterdir():
            path.unlink()
        result = self.run_launcher("linux", "--backend", "--check", OMNIVOX_WSL_AUDIO_OUTPUT="pulse")
        self.assertEqual(result["env"]["OMNIVOX_AUDIO_OUTPUT"], "pulse")
        self.assertIsNone(result["env"]["ALSA_CONFIG_PATH"])
        self.assertIsNone(result["env"]["PULSE_LATENCY_MSEC"])
        with patch.dict(os.environ, {"OMNIVOX_WSL_AUDIO_OUTPUT": "pulse"}):
            report = wsl_audio.report(self.config, self.directory)
        self.assertIsNone(report["alsa_plugin_sha256"])
        self.assertIn("native libpulse", report["runtimes"]["linux"]["configured_audio_path"])

    def test_default_clears_inherited_latency_and_keeps_linux_runtime(self) -> None:
        result = self.run_launcher(
            "linux", "--backend", "--engine", "espeak", "--check",
            OMNIVOX_WSL_LATENCY_MS="default", PULSE_LATENCY_MSEC="20",
            OMNIVOX_RHVOICE_LIBRARY="/usr/lib/libRHVoice.so",
        )
        self.assertEqual(result["args"], ["--engine", "espeak", "--check"])
        self.assertIsNone(result["env"]["PULSE_LATENCY_MSEC"])
        self.assertEqual(result["env"]["OMNIVOX_RHVOICE_LIBRARY"], "/usr/lib/libRHVoice.so")

    def test_windows_preserves_auto_selection_and_vendor_environment(self) -> None:
        result = self.run_launcher(
            "windows", "--backend", "--version-extra",
            OMNIVOX_PROGRAM=str(self.linux), OMNIVOX_RHVOICE_LIBRARY=r"C:\Voice\rhvoice.dll",
        )
        self.assertEqual(result["env"]["OMNIVOX_PROGRAM"], "")
        self.assertEqual(result["env"]["OMNIVOX_RHVOICE_LIBRARY"], r"C:\Voice\rhvoice.dll")
        self.assertEqual(result["args"], ["--version-extra"])

    def test_rejects_wrong_platform_after_auto_selection_changes(self) -> None:
        with patch.dict(os.environ, {"FAKE_WINDOWS_PROGRAM": str(self.linux)}):
            env, _ = wsl_audio.environment(self.config, self.directory, "windows")
            with self.assertRaisesRegex(ValueError, "wrong platform"):
                wsl_audio.resolve_program(self.config, env, "windows")

    def test_prepare_refuses_existing_directory_without_overwriting(self) -> None:
        marker = self.directory / "preserve"
        marker.write_text("existing work")
        with self.assertRaisesRegex(ValueError, "existing trial directory"):
            wsl_audio.prepare(self.args)
        self.assertEqual(marker.read_text(), "existing work")

    def test_missing_plugin_or_socket_is_actionable(self) -> None:
        with self.assertRaisesRegex(ValueError, "ALSA PulseAudio plugins are missing"):
            wsl_audio.plugin_directory(self.root)
        with self.assertRaisesRegex(ValueError, "socket is unavailable"):
            wsl_audio.validate_pulse_server(f"unix:{self.root / 'absent'}")
        with self.assertRaisesRegex(ValueError, "absolute unix"):
            wsl_audio.validate_pulse_server("tcp:example.org")

    def test_plugin_discovery_prefers_native_multiarch(self) -> None:
        with patch.object(wsl_audio.sysconfig, "get_config_var", return_value="native-linux"), \
             patch.object(Path, "is_file", return_value=True):
            self.assertEqual(wsl_audio.plugin_directory(None),
                             Path("/usr/lib/native-linux/alsa-lib").resolve())

    def test_mixed_voice_lists_and_unc_paths_are_removed_without_mutating_parent(self) -> None:
        original = {
            "OMNIVOX_FLITE_VOICES": r"/home/voices/local.flitevox;C:\Voices\other.flitevox",
            "OMNIVOX_PIPER_MODEL": r"\\server\voices\voice.onnx",
            "OMNIVOX_RHVOICE_RESOURCES": "/home/voices/resources",
            "OMNIVOX_ECI_LIBRARY": r"C:\Voice\ECI.DLL",
            "OMNIVOX_DECTALK_LIBRARY": "/usr/local/lib/libtts_us.so",
            "OMNIVOX_DECTALK_DICTIONARY": "/opt/dectalk/dic/dtalk_us.dic",
        }
        env, removed = wsl_audio.environment(self.config, self.directory, "linux", original)
        self.assertCountEqual(removed, ["OMNIVOX_FLITE_VOICES", "OMNIVOX_PIPER_MODEL",
                                        "OMNIVOX_ECI_LIBRARY"])
        self.assertEqual(env["OMNIVOX_RHVOICE_RESOURCES"], "/home/voices/resources")
        self.assertEqual(env["OMNIVOX_DECTALK_LIBRARY"], "/usr/local/lib/libtts_us.so")
        self.assertEqual(env["OMNIVOX_DECTALK_DICTIONARY"], "/opt/dectalk/dic/dtalk_us.dic")
        self.assertEqual(len(original), 6)

    def test_report_identifies_both_binaries_without_claiming_acoustic_measurement(self) -> None:
        result = self.run_launcher("linux", "--report")
        self.assertTrue(result["versions_match"])
        self.assertFalse(result["acoustic_onset_measured"])
        self.assertFalse(result["stop_to_silence_measured"])
        self.assertEqual(result["runtimes"]["linux"]["sha256"], wsl_audio.sha256(self.linux))

    def test_pactl_filters_other_clients_and_identifiers(self) -> None:
        response = [
            {"buffer_latency_usec": 50000, "sink_latency_usec": 90000,
             "properties": {"application.process.id": "42", "application.user": "private"}},
            {"buffer_latency_usec": 100, "properties": {"application.process.id": "43"}},
        ]
        with patch.object(wsl_audio.subprocess, "run", return_value=
                          subprocess.CompletedProcess([], 0, json.dumps(response))):
            entries = wsl_audio.sink_inputs({}, 42)
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]["buffer_latency_usec"], 50000)
        self.assertNotIn("properties", entries[0])

    def test_probe_retains_samples_and_owns_its_processes(self) -> None:
        output = self.root / "evidence"
        with patch.dict(os.environ, {"FAKE_IDLE": "1"}), \
             patch.object(wsl_audio.shutil, "which", return_value="/bin/true"), \
             patch.object(wsl_audio, "sink_inputs", return_value=[
                 {"buffer_latency_usec": 50000, "sink_latency_usec": 90000}]):
            self.assertTrue(wsl_audio.probe(self.config, self.directory, output, 2))
        results = json.loads((output / "buffers.json").read_text())
        self.assertEqual([run["pulse_latency_ms"] for run in results["runs"]], ["default", "50"])
        for run in results["runs"]:
            self.assertEqual(len(run["samples"]), 2)
            self.assertTrue(run["shutdown_within_deadline"])
            self.assertEqual(run["exit_code"], 0)
        self.assertFalse(results["acoustic_onset_measured"])
        self.assertFalse(results["stop_to_silence_measured"])

    def test_probe_records_failure_and_does_not_report_success(self) -> None:
        output = self.root / "failed-evidence"
        with patch.dict(os.environ, {"FAKE_IDLE": "1"}), \
             patch.object(wsl_audio.shutil, "which", return_value="/bin/true"), \
             patch.object(wsl_audio, "sink_inputs", side_effect=RuntimeError("pactl failed")):
            self.assertFalse(wsl_audio.probe(self.config, self.directory, output, 1))
        results = json.loads((output / "buffers.json").read_text())
        for run in results["runs"]:
            self.assertEqual(run["error"], "pactl failed")
            self.assertFalse(run["passed"])
            self.assertEqual(run["exit_code"], 0)

    def test_missing_pulse_measurements_fail_instead_of_becoming_zero_latency(self) -> None:
        response = [{"properties": {"application.process.id": "42"}}]
        with patch.object(wsl_audio.subprocess, "run", return_value=
                          subprocess.CompletedProcess([], 0, json.dumps(response))):
            with self.assertRaisesRegex(ValueError, "valid buffer_latency_usec"):
                wsl_audio.sink_inputs({}, 42)

    def test_stalled_process_is_killed_after_shutdown_deadline(self) -> None:
        process = subprocess.Popen(
            [sys.executable, "-c",
             "import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(60)"],
            stdin=subprocess.PIPE, start_new_session=True,
        )
        try:
            self.assertFalse(wsl_audio.stop_process(process))
            self.assertEqual(process.returncode, -signal.SIGKILL)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=5)
            process.stdin.close()


class PulseHealthTests(unittest.TestCase):
    def test_reachable_checks_selected_server_without_retaining_private_output(self) -> None:
        env = {"PULSE_SERVER": "unix:/selected/server", "PATH": "/tools"}
        with patch.object(wsl_audio.shutil, "which", return_value="/tools/pactl"), \
             patch.object(wsl_audio.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)) as run:
            result = wsl_audio.pulse_health(env)
        self.assertEqual(result["status"], "reachable")
        self.assertFalse(result["playback_verified"])
        self.assertEqual(run.call_args.args[0], ["/tools/pactl", "--server=unix:/selected/server", "info"])
        self.assertEqual(run.call_args.kwargs["env"], env)
        self.assertEqual(run.call_args.kwargs["timeout"], 3)
        self.assertEqual(run.call_args.kwargs["stdout"], subprocess.DEVNULL)
        self.assertEqual(run.call_args.kwargs["stderr"], subprocess.DEVNULL)

    def test_connection_failure_is_distinct_from_a_timeout(self) -> None:
        with patch.object(wsl_audio.shutil, "which", return_value="/tools/pactl"), \
             patch.object(wsl_audio.subprocess, "run", return_value=subprocess.CompletedProcess([], 1)):
            result = wsl_audio.pulse_health({})
        self.assertEqual(result["status"], "unavailable")
        self.assertEqual(result["exit_code"], 1)

    def test_missing_pactl_does_not_claim_a_healthy_server(self) -> None:
        with patch.object(wsl_audio.shutil, "which", return_value=None), \
             patch.object(wsl_audio.subprocess, "run") as run:
            result = wsl_audio.pulse_health({})
        self.assertEqual(result["status"], "unverified")
        run.assert_not_called()

    @unittest.skipUnless(os.name == "posix", "fake executable requires POSIX")
    def test_hung_probe_times_out_and_reaps_only_its_child(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pid_file = root / "probe.pid"
            pactl = root / "pactl"
            pactl.write_text(
                f"#!{sys.executable}\nimport os, pathlib, time\n"
                f"pathlib.Path({str(pid_file)!r}).write_text(str(os.getpid()))\n"
                "time.sleep(60)\n"
            )
            pactl.chmod(0o700)
            result = wsl_audio.pulse_health({"PATH": directory}, timeout=0.5)
            self.assertEqual(result["status"], "timeout")
            self.assertLess(result["elapsed_ms"], 2000)
            with self.assertRaises(ProcessLookupError):
                os.kill(int(pid_file.read_text()), 0)


class LatencyTests(unittest.TestCase):
    def test_rejects_stalling_and_unbounded_requests(self) -> None:
        for value in ("0", "20", "49", "-50", "1001", "nan", "1e2", "9" * 100, ""):
            with self.subTest(value=value), self.assertRaises(ValueError):
                wsl_audio.latency(value)
        for value, expected in (("default", "default"), ("50", "50"),
                                ("0050", "50"), ("1000", "1000")):
            self.assertEqual(wsl_audio.latency(value), expected)


if __name__ == "__main__":
    unittest.main()
