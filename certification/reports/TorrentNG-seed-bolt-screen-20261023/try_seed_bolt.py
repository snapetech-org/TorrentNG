"""Task-local BOLT exploration; run only after the matched benchmark stops."""
import hashlib,json,os,subprocess,sys
from pathlib import Path
root=Path('/workspace/scratch/ca998092d7e5'); tools_repo=root/'TorrentNG'; repo=root/'TorrentNG-main-control'
sys.path.insert(0,str(tools_repo/'scripts'))
from native_seed_pgo import validate_profile,workload_args
from seed_efficiency_tools import manifest,sha256
profile=root/'candidate-pgo-training/seed.profdata'
target=root/'main-bolt-build'
out=root/'main-bolt-experiment'
out.mkdir(exist_ok=False)
log=(out/'build.log').open('w')
env={**os.environ,'PATH':'/root/.cargo/bin:'+os.environ['PATH'],'RUSTFLAGS':f'-Cprofile-use={profile}'}
validate_profile(profile,repo)
def run(argv,**kwargs):
 print('RUN',*map(str,argv),flush=True)
 subprocess.run(list(map(str,argv)),cwd=repo,env=env,check=True,stdout=kwargs.pop('stdout',log),stderr=log,**kwargs)
run(['cargo','+1.97.0','rustc','--locked','--release','-p','torrentngd','--bin','torrentngd','--target-dir',target,'--','-Clink-arg=-Wl,--emit-relocs'])
binary=out/'torrentngd.pgo-relocs'; binary.write_bytes((target/'release/torrentngd').read_bytes()); binary.chmod(0o755)
metadata=manifest(binary,repo,profile); metadata['rustflags']=env['RUSTFLAGS']; metadata['binary_link_args']=['-Clink-arg=-Wl,--emit-relocs']
(out/'pgo-relocs.manifest.json').write_text(json.dumps(metadata,indent=2)+'\n')
instrumented=out/'torrentngd.instrumented'; raw=out/'raw'; raw.mkdir()
run(['llvm-bolt-20',binary,'-instrument','-o',instrumented,'-instrumentation-file='+str(raw/'profile.fdata'),'-instrumentation-file-append-pid','-instrumentation-binpath='+str(instrumented)])
corpus=json.loads((tools_repo/'certification/benchmarks/seed-efficiency-corpus.json').read_text())['training']
for i,workload in enumerate(corpus):
 run([sys.executable,tools_repo/'scripts/native_seed_cpu_ram_probe.py','--client','training=torrentngd:'+str(instrumented),'--allow-single-client','--repeats','1','--output',out/f'training-{i}','--client-cpus','0-1',*workload_args(workload)])
profiles=sorted(raw.glob('*.fdata*')); assert profiles,'BOLT did not save profiles'
merged=out/'seed.fdata'
with merged.open('w') as f: run(['merge-fdata-20',*profiles],stdout=f)
optimized=out/'torrentngd.bolt'
run(['llvm-bolt-20',binary,'-o',optimized,'-data='+str(merged),'-reorder-blocks=ext-tsp','-reorder-functions=cdsort','-split-functions','-split-all-cold','-split-eh','-dyno-stats'])
record={'format':'torrentng-bolt-experiment-v1','input_binary_sha256':sha256(binary),'instrumented_binary_sha256':sha256(instrumented),'profile_sha256':sha256(merged),'output_binary_sha256':sha256(optimized),'training':corpus,'bolt_version':subprocess.check_output(['llvm-bolt-20','--version'],text=True),'tool_sha256':sha256('/usr/lib/llvm-20/bin/llvm-bolt'),'pgo_profile_sha256':sha256(profile)}
(out/'bolt-profile.manifest.json').write_text(json.dumps(record,indent=2)+'\n')
metadata['binary']=str(optimized);metadata['binary_sha256']=sha256(optimized);metadata['post_link']=record
(out/'bolt.manifest.json').write_text(json.dumps(metadata,indent=2)+'\n')
print('READY',optimized,flush=True)
