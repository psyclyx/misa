"""A/B startup probe using temporary, freshly translated Lua sources."""
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import tempfile
import time

root = Path(__file__).resolve().parent.parent
samples = {"source": [], "precompiled": []}
with tempfile.TemporaryDirectory(prefix="misa-startup-") as directory:
    command = ["luajit", "tools/compile-fennel.lua"]
    paths = sorted((root / "extensions").rglob("*.fnl"))
    paths += [root / "src/lua_runtime" / name for name in
              ["state.fnl", "subscriptions.fnl", "framework.fnl"]]
    for path in paths:
        relative = path.relative_to(root)
        output = Path(directory) / relative.with_suffix(".lua")
        output.parent.mkdir(parents=True, exist_ok=True)
        command.extend([str(relative), str(output)])
    subprocess.run(command, cwd=root, check=True, capture_output=True, timeout=60)
    for iteration in range(10):
        order = ["source", "precompiled"] if iteration % 2 == 0 else ["precompiled", "source"]
        for mode in order:
            environment = os.environ.copy()
            environment.pop("MISA_STARTUP_PRECOMPILED", None)
            if mode == "precompiled":
                environment["MISA_STARTUP_PRECOMPILED"] = directory
            start = time.perf_counter()
            result = subprocess.run(["luajit", "benchmarks/startup.lua"], cwd=root,
                                    env=environment, check=True, capture_output=True,
                                    text=True, timeout=30)
            sample = json.loads(result.stdout)
            sample["wall"] = time.perf_counter() - start
            samples[mode].append(sample)
oracles = {sample["oracle"] for group in samples.values() for sample in group}
if len(oracles) != 1:
    raise RuntimeError("source and generated startup semantics differ")
summary = {}
for mode, group in samples.items():
    summary[mode] = {
        phase: {"median_ms": statistics.median(s[phase] for s in group) * 1000,
                "best_ms": min(s[phase] for s in group) * 1000}
        for phase in ["compile", "load", "setup", "dispatch", "total", "wall"]
    }
print(json.dumps({"summary": summary,
                  "oracle_sha256": hashlib.sha256(next(iter(oracles)).encode()).hexdigest(),
                  "raw_seconds": {mode: [{key: s[key] for key in ["total", "wall"]}
                                         for s in group] for mode, group in samples.items()}}, indent=2))
