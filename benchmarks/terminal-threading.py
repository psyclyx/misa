#!/usr/bin/env python3
"""Compare settled-frame PTY latency with alternating executable order.

Each sample is one invocation of the existing three-trial correctness regression.
Reported medians and maxima are milliseconds across those three trials, not
individual input samples. Every invocation must pass before emitting a row.
"""
import argparse
import csv
import re
import subprocess
import sys
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('baseline', type=Path)
parser.add_argument('candidate', type=Path)
parser.add_argument('--samples', type=int, default=10)
args = parser.parse_args()
if args.samples < 1:
    parser.error('--samples must be positive')
root = Path(__file__).resolve().parent.parent
executables = {'baseline': args.baseline.resolve(), 'candidate': args.candidate.resolve()}
writer = csv.writer(sys.stdout, lineterminator='\n')
writer.writerow(['round', 'variant', 'trial_median_ms', 'trial_max_ms'])
for sample in range(args.samples):
    order = ['baseline', 'candidate'] if sample % 2 == 0 else ['candidate', 'baseline']
    for variant in order:
        result = subprocess.run(
            [sys.executable, str(root / 'tests/settled-frames.py'), str(executables[variant])],
            cwd=root, capture_output=True, text=True, timeout=30, check=True,
        )
        match = re.search(r'latency median=([0-9.]+)ms max=([0-9.]+)ms', result.stdout)
        if match is None:
            raise RuntimeError(f'unrecognized regression output: {result.stdout!r}')
        writer.writerow([sample + 1, variant, *match.groups()])
        sys.stdout.flush()
