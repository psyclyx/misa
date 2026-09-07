"""Sequential startup phase probe; does not execute native effects."""
import argparse
import hashlib
import json
from pathlib import Path
import statistics
import subprocess
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--samples", type=int, default=10)
args = parser.parse_args()
if args.samples < 6:
    parser.error("use at least six samples to check semantic repeatability")
root = Path(__file__).resolve().parent.parent
samples = []
for _ in range(args.samples):
    start = time.perf_counter()
    process = subprocess.run(["luajit", "benchmarks/startup.lua"], cwd=root,
                             capture_output=True, text=True, check=True, timeout=30)
    sample = json.loads(process.stdout)
    sample["wall"] = time.perf_counter() - start
    samples.append(sample)
oracles = {sample["oracle"] for sample in samples}
if len(oracles) != 1:
    raise RuntimeError("startup semantics changed across identical runs")
phases = ["bootstrap", "compile", "load", "setup", "dispatch", "total", "wall"]
summary = {
    phase: {"median_ms": statistics.median(s[phase] for s in samples) * 1000,
            "best_ms": min(s[phase] for s in samples) * 1000}
    for phase in phases
}
paths = [row["path"] for row in samples[0]["rows"]]
compile_times = sorted([
    {"path": path, "median_ms": statistics.median(
        next(row["compile"] for row in sample["rows"] if row["path"] == path)
        for sample in samples) * 1000} for path in paths
], key=lambda row: row["median_ms"], reverse=True)
print(json.dumps({
    "samples": len(samples), "phase_summary": summary,
    "oracle_sha256": hashlib.sha256(next(iter(oracles)).encode()).hexdigest(),
    "compilation": compile_times,
    "raw_seconds": [{phase: sample[phase] for phase in phases} for sample in samples],
}, indent=2))
