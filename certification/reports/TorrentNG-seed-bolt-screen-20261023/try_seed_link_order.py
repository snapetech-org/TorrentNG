"""Use BOLT only to plan function order, then link the original machine code."""
import json,os,re,subprocess,sys
from pathlib import Path
root=Path('/workspace/scratch/ca998092d7e5');tools_repo=root/'TorrentNG';source=root/'TorrentNG-main-control';out=root/'main-bolt-experiment';target=root/'main-bolt-build'
sys.path.insert(0,str(tools_repo/'scripts'))
from seed_efficiency_tools import manifest,sha256
profile=root/'candidate-pgo-training/seed.profdata';raw_order=out/'function-order.txt';order=out/'lld-function-order.txt'
log=(out/'link-order-build.log').open('w')
env={**os.environ,'PATH':'/root/.cargo/bin:'+os.environ['PATH'],'RUSTFLAGS':f'-Cprofile-use={profile}'}
def run(args):
 print('RUN',*map(str,args),flush=True)
 subprocess.run(list(map(str,args)),cwd=source,env=env,stdout=log,stderr=log,check=True)
run(['llvm-bolt-20',out/'torrentngd.pgo-relocs','-o',out/'layout-planner.bolt','-data='+str(out/'seed.fdata'),'-reorder-blocks=ext-tsp','-reorder-functions=cdsort','-generate-function-order='+str(raw_order)])
assert raw_order.is_file() and raw_order.stat().st_size>0
# BOLT adds /N to disambiguate local symbols. LLD needs their original ELF names.
nm=subprocess.check_output(['nm','--defined-only',str(out/'torrentngd.pgo-relocs')],text=True)
symbols={line.split(None,2)[2] for line in nm.splitlines() if len(line.split(None,2))==3}
normalized=list(dict.fromkeys(re.sub(r'/\d+$','',line) for line in raw_order.read_text().splitlines()))
assert normalized and all(name in symbols for name in normalized)
order.write_text('\n'.join(normalized)+'\n')
run(['cargo','+1.97.0','rustc','--locked','--release','-p','torrentngd','--bin','torrentngd','--target-dir',target,'--','-Clink-arg=-Wl,--emit-relocs','-Clink-arg=-Wl,--symbol-ordering-file='+str(order)])
binary=out/'torrentngd.link-order';binary.write_bytes((target/'release/torrentngd').read_bytes());binary.chmod(0o755)
m=manifest(binary,source,profile);m['rustflags']=env['RUSTFLAGS'];m['binary_link_args']=['-Clink-arg=-Wl,--emit-relocs','-Clink-arg=-Wl,--symbol-ordering-file='+str(order)]
m['link_order']={'order_sha256':sha256(order),'order_file':str(order),'training_input_binary_sha256':sha256(out/'torrentngd.pgo-relocs'),'training_profile_sha256':sha256(out/'seed.fdata'),'planner':'LLVM BOLT 20.1.2 cdsort function order; rewritten planner output is not executed','planner_sha256':sha256('/usr/lib/llvm-20/bin/llvm-bolt')}
(out/'link-order.manifest.json').write_text(json.dumps(m,indent=2)+'\n');print('READY',binary,flush=True)
