#!/usr/bin/env python3
"""Interleave saved extension trees using one executable and verify paint parity."""
import argparse
import csv
import io
import subprocess
import sys
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('baseline', type=Path)
parser.add_argument('candidate', type=Path)
parser.add_argument('--models', type=int, nargs='+', default=[1000, 3000])
parser.add_argument('--samples-per-block', type=int, default=5)
args = parser.parse_args()
if args.samples_per_block < 1:
    parser.error('--samples-per-block must be positive')
root = Path(__file__).resolve().parent.parent
writer = None
oracles = {}
for models in args.models:
    for block, variant in enumerate(('baseline', 'candidate', 'candidate', 'baseline'), 1):
        tree = getattr(args, variant)
        result = subprocess.run([
            sys.executable, str(root / 'benchmarks/picker-key-repeat.py'), str(args.binary.resolve()),
            '--extension-dir', str(tree.resolve()), '--models', str(models),
            '--samples', str(args.samples_per_block), '--warmups', '6',
        ], cwd=root, check=True, capture_output=True, text=True)
        for sample in csv.DictReader(io.StringIO(result.stdout)):
            key = (sample['models'], sample['rate_hz'], sample['final_row'])
            if key in oracles:
                assert oracles[key] == sample['frame_sha256'], f'paint parity mismatch: {variant}, {key}'
            else:
                oracles[key] = sample['frame_sha256']
            row = {'variant': variant, 'block': block, **sample}
            if writer is None:
                writer = csv.DictWriter(sys.stdout, fieldnames=list(row), lineterminator='\n')
                writer.writeheader()
            writer.writerow(row)
        sys.stdout.flush()
