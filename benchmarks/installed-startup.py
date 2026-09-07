"""Compare one installed binary's generated catalog with source overrides."""
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
args = parser.parse_args()
binary = args.binary.resolve()
root = Path(__file__).resolve().parent.parent
samples = {"source": [], "installed": []}
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
        modes = ["source", "installed"] if iteration % 2 == 0 else ["installed", "source"]
        for mode in modes:
            environment = base.copy()
            if mode == "source":
                environment["MISA_EXTENSION_DIR"] = str(root / "extensions")
            start = time.perf_counter()
            result = subprocess.run([str(binary), "--config", str(root / "config/default.json")],
                                    env=environment, input=b"", capture_output=True,
                                    check=True, timeout=30)
            samples[mode].append(time.perf_counter() - start)
            assert result.stdout == b"misa> enter a prompt:\n", result.stdout
            assert not result.stderr, result.stderr
            outputs.add(result.stdout)
assert len(outputs) == 1
print("binary_sha256", hashlib.sha256(binary.read_bytes()).hexdigest())
for mode, values in samples.items():
    print(mode, "median_ms", statistics.median(values) * 1000, "best_ms", min(values) * 1000)
    print("raw_seconds", values)
