#!/usr/bin/env python3
"""Build private, pinned MBROLA prototype inputs; never alter release bundles."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
WORK = ROOT / "target/mbrola-prototype"
MBROLA = "274dead162f2826dc38c208fba92efeddb724c33"
VOICES = "fe05a0ccef6a941207fd6aaad0b31294a1f93a51"
INPUTS = {
    "mbrola.tar.gz": (f"https://codeload.github.com/numediart/MBROLA/tar.gz/{MBROLA}",
                      "eb8b9da0fe3dfd3780e5a1b391189a334f96f2a5741771ed5bed819d4451c792"),
    "espeak.crate": ("https://static.crates.io/crates/espeak-rs-sys/espeak-rs-sys-0.1.9.crate",
                     "eeb333310ae915d57961a6bbc67f53ef7b313a9ee80d393d55c0f27f1b65c848"),
    "en1": (f"https://raw.githubusercontent.com/numediart/MBROLA-voices/{VOICES}/data/en1/en1",
            "edb8eaae6f0e38493d88ed627518632e6ff8a3843bcf08474a1a70aa786fd99f"),
    "en1-README.txt": (f"https://raw.githubusercontent.com/numediart/MBROLA-voices/{VOICES}/data/en1/README.txt",
                       "aea83c2f1264bd1f769c9074a496015a87e0a8fcab6fd4cf700c1cdf0596c128"),
    "voices-LICENSE.md": (f"https://raw.githubusercontent.com/numediart/MBROLA-voices/{VOICES}/LICENSE.md",
                          "894056225f1bfb82cf85630ff6d0e2866f04671ca1d7576da254813ed866b81f"),
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(*args, cwd=None):
    print("+", *map(str, args), flush=True)
    subprocess.run(list(map(str, args)), cwd=cwd or ROOT, check=True)


def inputs():
    cache = WORK / "inputs"
    cache.mkdir(parents=True, exist_ok=True)
    for name, (url, expected) in INPUTS.items():
        path = cache / name
        if not path.exists():
            with urllib.request.urlopen(url, timeout=60) as response:
                data = response.read(64 * 1024 * 1024 + 1)
            if len(data) > 64 * 1024 * 1024 or hashlib.sha256(data).hexdigest() != expected:
                raise ValueError(f"invalid download: {name}")
            path.write_bytes(data)
        if digest(path) != expected:
            raise ValueError(f"input hash mismatch: {path}")
    return cache


def source(cache, archive, dest, child):
    # Only the ignored generated copy receives overlays; pristine inputs stay intact.
    dest.mkdir(parents=True, exist_ok=True)
    with tarfile.open(cache / archive) as stream:
        stream.extractall(dest, filter="data")
    return dest / child


# Keep just the reviewed English profiles. Do not scan and hash hundreds of unused
# language/voice files on every utterance, especially on native Windows.
FRONTEND_DATA = (
    "phontab", "phondata", "phonindex", "intonations", "en_dict",
    "voices/mb/mb-en1", "mbrola_ph/en1_phtrans",
    "voices/mb/mb-us1", "mbrola_ph/us_phtrans",
    "voices/mb/mb-us2",
    "voices/mb/mb-us3", "mbrola_ph/us3_phtrans",
)


def stage_frontend_data(source, destination):
    # Prepare the complete replacement before touching generated staging data.
    # Rebuilding must also remove languages left by an earlier full-data build.
    with tempfile.TemporaryDirectory(prefix="mbrola-data-", dir=destination.parent) as temporary:
        prepared = Path(temporary)
        for name in FRONTEND_DATA:
            output = prepared / name
            output.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source / name, output)
        if destination.exists():
            shutil.rmtree(destination)
        shutil.copytree(prepared, destination)


def build_native(platform, cache):
    work = WORK / platform
    work.mkdir(parents=True, exist_ok=True)
    src = source(cache, "espeak.crate", work / "frontend-source", "espeak-rs-sys-0.1.9/espeak-ng")
    shutil.copy2(ROOT / "tools/mbrola/frontend-only.c", src / "src/libespeak-ng/mbrowrap.c")
    build = work / "frontend-build"
    options = ["-DCMAKE_BUILD_TYPE=Release", "-DBUILD_SHARED_LIBS=OFF", "-DENABLE_TESTS=OFF",
               "-DUSE_ASYNC=OFF", "-DUSE_LIBPCAUDIO=OFF", "-DUSE_LIBSONIC=OFF",
               "-DSONIC_LIB=m", "-DSONIC_INC=/usr/include", "-DUSE_MBROLA=ON",
               "-DMBROLA_BIN=frontend-only", "-DUSE_SPEECHPLAYER=OFF"]
    if platform == "windows":
        options += ["-DCMAKE_SYSTEM_NAME=Windows", "-DCMAKE_C_COMPILER=x86_64-w64-mingw32-gcc",
                    "-DCMAKE_CXX_COMPILER=x86_64-w64-mingw32-g++", "-DCMAKE_EXE_LINKER_FLAGS=-static"]
    run("cmake", "-S", src, "-B", build, *options)
    run("cmake", "--build", build, "--parallel", "4", "--target",
        "espeak-ng-bin" if platform == "windows" else "data")
    msrc = source(cache, "mbrola.tar.gz", work / "runtime-source", f"MBROLA-{MBROLA}")
    # Upstream stdout defaults to text on Windows; PCM must preserve every byte.
    main = msrc / "Standalone/synth.c"
    text = main.read_text()
    needle = "output_file=stdout;"
    assert text.count(needle) == 1
    text = text.replace(needle, '{' + needle + '\n#ifdef _WIN32\n_setmode(_fileno(stdout), _O_BINARY);\n#endif\n}')
    main.write_text('#ifdef _WIN32\n#include <io.h>\n#include <fcntl.h>\n#endif\n' + text)
    options = ["EXT_CFLAGS=-O2"]
    if platform == "windows":
        options += ["CC=x86_64-w64-mingw32-gcc", "EXT_CFLAGS=-O2 -DLITTLE_ENDIAN", "LDFLAGS=-static"]
    run("make", "-j4", *options, cwd=msrc)
    stage = work / "runtime"
    stage.mkdir(exist_ok=True)
    suffix = ".exe" if platform == "windows" else ""
    shutil.copy2(build / f"src/espeak-ng{suffix}", stage / f"frontend{suffix}")
    runtime = msrc / "Bin/mbrola"
    if not runtime.exists():
        runtime = runtime.with_suffix(".exe")
    shutil.copy2(runtime, stage / f"mbrola{suffix}")
    # Generated eSpeak data is architecture-independent, as in existing companions.
    data = WORK / "linux/frontend-build/espeak-ng-data"
    stage_frontend_data(data, stage / "espeak-ng-data")
    (stage / "espeak-ng-data/mbrola").mkdir(exist_ok=True)
    shutil.copy2(cache / "en1", stage / "espeak-ng-data/mbrola/en1")
    notices = stage / "notices"
    notices.mkdir(exist_ok=True)
    for name in ("en1-README.txt", "voices-LICENSE.md"):
        shutil.copy2(cache / name, notices / name)
    shutil.copy2(msrc / "LICENSE", notices / "MBROLA-AGPL-3.0.txt")
    shutil.copy2(src / "src/ucd-tools/COPYING", notices / "eSpeak-GPL-3.0.txt")
    shutil.copy2(src / "src/ucd-tools/COPYING.UCD", notices / "Unicode-Data-License.txt")
    shutil.copy2(ROOT / "tools/mbrola/frontend-only.c", notices / "frontend-only.c")
    manifest = dict(schema_version=1, voice_id="mbrola:v1/mb-en1/en1", sample_rate=16000,
                    frontend=f"frontend{suffix}", runtime=f"mbrola{suffix}",
                    database="espeak-ng-data/mbrola/en1",
                    files={p.relative_to(stage).as_posix(): digest(p)
                           for p in sorted(stage.rglob("*")) if p.is_file()
                           and p.name not in ("prototype.json", "SHA256SUMS", "SOURCE-PROVENANCE.json", f"omnivox-mbrola-helper{suffix}")},
                    sources={name: dict(url=url, sha256=sha) for name, (url, sha) in INPUTS.items()},
                    builder_sha256=digest(Path(__file__)),
                    frontend_overlay_sha256=digest(ROOT / "tools/mbrola/frontend-only.c"))
    (stage / "prototype.json").write_text(json.dumps(manifest, indent=2) + "\n")
    return stage


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=["linux", "windows"], default="linux")
    parser.add_argument("--native-only", action="store_true", help="Prepare native inputs before the Rust helper")
    args = parser.parse_args()
    cache = inputs()
    if args.platform == "windows" and not (WORK / "linux/frontend-build/espeak-ng-data/en_dict").exists():
        build_native("linux", cache)
    stage = build_native(args.platform, cache)
    if not args.native_only:
        target = "x86_64-pc-windows-gnu" if args.platform == "windows" else "x86_64-unknown-linux-gnu"
        run("cargo", "+1.97.1", "build", "--locked", "--release", "--target", target,
            "-p", "omnivox-mbrola-helper")
        name = "omnivox-mbrola-helper" + (".exe" if args.platform == "windows" else "")
        shutil.copy2(ROOT / "target" / target / "release" / name, stage / name)
        provenance = dict(schema_version=1, target=target,
                          artifact=f"omnivox-mbrola-companion-development-{target}",
                          prototype_sha256=digest(stage / "prototype.json"),
                          source_commit=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                          tracked_diff_sha256=hashlib.sha256(subprocess.check_output(["git", "diff", "HEAD", "--"], cwd=ROOT)).hexdigest())
        (stage / "SOURCE-PROVENANCE.json").write_text(json.dumps(provenance, indent=2) + "\n")
        (stage / "SHA256SUMS").write_text("".join(
            f"{digest(p)}  {p.relative_to(stage).as_posix()}\n"
            for p in sorted(stage.rglob("*")) if p.is_file() and p.name != "SHA256SUMS"))
    print(f"Private prototype staged at {stage}")


if __name__ == "__main__":
    main()
