"""Compare installed/source loading, or two binaries on the installed catalog."""
import argparse
import hashlib
import os
from pathlib import Path
import statistics
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("binary", type=Path)
parser.add_argument("--baseline", type=Path,
                    help="compare both binaries using their installed catalogs instead of source overrides")
args = parser.parse_args()
binary = args.binary.resolve()
root = Path(__file__).resolve().parent.parent
executables = ({"baseline": args.baseline.resolve(), "candidate": binary} if args.baseline
               else {"source": binary, "installed": binary})
samples = {mode: [] for mode in executables}
outputs = set()
with tempfile.TemporaryDirectory(prefix="misa-installed-startup-") as directory:
    base = os.environ.copy()
    for key in ["MISA_CONFIG", "MISA_EXTENSION_DIR", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"]:
        base.pop(key, None)
    # Isolate credentials/state and prevent launching provider CLIs. Native
    # status checks see empty storage; no live account is part of this workload.
    base.update(MISA_AUTH_FILE=directory + "/auth.json", MISA_STATE_FILE=directory + "/state.json",
                XDG_CONFIG_HOME=directory, XDG_STATE_HOME=directory, PATH="")
    for iteration in range(10):
        modes = list(executables)
        if iteration % 2:
            modes.reverse()
        for mode in modes:
            environment = base.copy()
            if mode == "source":
                environment["MISA_EXTENSION_DIR"] = str(root / "extensions")
            start = time.perf_counter()
            result = subprocess.run([str(executables[mode])],
                                    env=environment, input=b"", capture_output=True,
                                    check=True, timeout=30)
            samples[mode].append(time.perf_counter() - start)
            assert result.stdout == b"misa> enter a prompt:\n", result.stdout
            assert not result.stderr, result.stderr
            outputs.add(result.stdout)
assert len(outputs) == 1
for mode, executable in executables.items():
    print(mode, "binary_sha256", hashlib.sha256(executable.read_bytes()).hexdigest())
for mode, values in samples.items():
    print(mode, "median_ms", statistics.median(values) * 1000, "best_ms", min(values) * 1000)
    print("raw_seconds", values)
