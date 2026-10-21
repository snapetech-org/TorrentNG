#!/usr/bin/env python3
"""Run verified native-daemon comparisons sequentially, rotating run order."""
import argparse
import json
import subprocess
import sys
from pathlib import Path

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--case', action='append', required=True, help='label=/absolute/binary')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--fixtures', type=Path, required=True)
    parser.add_argument('--rounds', type=int, default=3)
    parser.add_argument('--mib', type=int, default=128)
    parser.add_argument('--refill', type=int, default=4)
    args=parser.parse_args()
    cases=[case.split('=',1) for case in args.case]
    args.output.mkdir(parents=True,exist_ok=False)
    for round_index in range(args.rounds):
        shift=round_index%len(cases)
        for label,binary in cases[shift:]+cases[:shift]:
            out=args.output/f'{round_index+1}-{label}'
            cmd=[sys.executable,str(Path(__file__).with_name('native_seed_probe.py')),
                 '--binary',str(Path(binary).resolve()),'--output',str(out.resolve()),
                 '--fixture-dir',str(args.fixtures.resolve()),'--mib',str(args.mib),
                 '--refill',str(args.refill),'--window','32','--rate-mib','4','--label',label]
            result=subprocess.run(cmd,capture_output=True,text=True)
            (args.output/f'{round_index+1}-{label}.log').write_text(result.stdout+result.stderr)
            if result.returncode:
                print(f'FAILED: {out}; see log',flush=True)
                return result.returncode
            row=json.loads((out/'result.json').read_text())
            summary={key:row[key] for key in ['label','cpu_s_per_gib','elapsed_seconds','peak_anon_kib','binary_sha256','payload_sha256']}
            summary.update(round=round_index+1,refill=args.refill)
            with (args.output/'summary.jsonl').open('a') as log:
                log.write(json.dumps(summary)+'\n')
            print(json.dumps(summary),flush=True)
    return 0

if __name__=='__main__':raise SystemExit(main())
