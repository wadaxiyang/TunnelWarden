"""Verify every vendored file against the pinned checkout and reviewed patch."""
import difflib
import json
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parent.parent
meta = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--locked", "--format-version", "1"], cwd=root))
core = next(p for p in meta["packages"] if p["name"] == "oryxis-core")
source = Path(core["manifest_path"]).parent.parent / "oryxis-ssh"
pin = "16ad2af83b424780ff60b6420a6c20a3b30c7143"
actual = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
if actual != pin:
    raise SystemExit(f"Wrong upstream: {actual}")
vendored = root / "vendor/oryxis-ssh"

def files(path):
    return {p.relative_to(path).as_posix(): p.read_text(encoding="utf-8")
            for p in path.rglob("*") if p.is_file()}

before, after = files(source), files(vendored)
diff = "".join("".join(difflib.unified_diff(
    before.get(name, "").splitlines(keepends=True),
    after.get(name, "").splitlines(keepends=True),
    fromfile=f"a/{name}", tofile=f"b/{name}"))
    for name in sorted(before.keys() | after.keys()))
patch = root / "upstream/oryxis-ssh-traffic.patch"
if "--write" in sys.argv:
    patch.write_text(diff, encoding="utf-8", newline="\n")
elif patch.read_text(encoding="utf-8") != diff:
    raise SystemExit("Vendored source differs from the reviewed Oryxis patch")
print(f"Verified {len(after)} vendored files against Oryxis {pin} and reviewed patch")
