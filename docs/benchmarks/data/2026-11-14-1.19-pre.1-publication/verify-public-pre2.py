import hashlib, json, re, subprocess, sys
from pathlib import Path
sys.path.insert(0, "tools")
from verify_release_asset_set import require_exact_names, checksum_names, expected_asset_names
version = "1.19.0-pre.2"
repo = "bartbunting/omnivox"
root = Path("/tmp/omnivox-public-pre2")
root.mkdir(exist_ok=True)
def gh(*args):
    return subprocess.check_output(["gh", *args])
release_bytes = gh("api", f"repos/{repo}/releases/tags/v{version}")
release = json.loads(release_bytes)
assert release["tag_name"] == f"v{version}" and not release["draft"] and release["prerelease"]
(root / "release.json").write_bytes(release_bytes)
latest_bytes = gh("api", f"repos/{repo}/releases/latest")
latest = json.loads(latest_bytes)
assert latest["tag_name"] == "v1.18.0", latest["tag_name"]
(root / "latest-stable.json").write_bytes(latest_bytes)
gh("release", "download", f"v{version}", "--repo", repo, "--pattern", "sha256sums.txt", "--dir", str(root))
assets = release["assets"]
require_exact_names([a["name"] for a in assets], version)
manifest = root / "sha256sums.txt"
names = checksum_names(manifest)
assert len(names) == len(set(names)) == 32
assert set(names) == expected_asset_names(version) - {"sha256sums.txt"}
hashes = {}
for line in manifest.read_text().splitlines():
    match = re.fullmatch(r"([0-9a-fA-F]{64})\s+\*?(.+)", line)
    assert match
    hashes[match[2]] = match[1].lower()
hashes["sha256sums.txt"] = hashlib.sha256(manifest.read_bytes()).hexdigest()
for asset in assets:
    assert asset["digest"] == "sha256:" + hashes[asset["name"]], asset["name"]
result = {"tag": release["tag_name"], "url": release["html_url"], "published_at": release["published_at"], "asset_count": len(assets), "api_digests_match_manifest": True, "latest_stable": latest["tag_name"]}
(root / "verification.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps(result, indent=2))
