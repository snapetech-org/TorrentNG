import json,os,subprocess,sys
from pathlib import Path
root=Path('/workspace/scratch/ca998092d7e5');repo=root/'TorrentNG';out=root/'main-bolt-experiment'
sys.path.insert(0,str(repo/'scripts'));from seed_efficiency_tools import paired_analysis
probe=repo/'scripts/native_seed_cpu_ram_probe.py'
env={**os.environ,'PYTHONPATH':str(root/'probe-python-deps')}
def run(args):
 print('RUN',*map(str,args),flush=True);subprocess.run(list(map(str,args)),env=env,check=True,cwd=repo)
run([sys.executable,probe,'--client','link-order=torrentngd:'+str(out/'torrentngd.link-order'),'--client-manifest','link-order='+str(out/'link-order.manifest.json'),'--allow-single-client','--repeats','1','--size',25165824,'--rate',8388608,'--window',64,'--peers',3,'--fixture-files',5,'--request-pattern','random','--cache-policy','drop-file-pages','--seed',20261027,'--client-cpus','0-1','--output',root/'link-order-functional-multifile'])
clients=[('pgo-relocs','torrentngd',out/'torrentngd.pgo-relocs',out/'pgo-relocs.manifest.json'),('pgo-bolt-4k','torrentngd',out/'torrentngd.bolt-4k',out/'bolt-4k.manifest.json'),('pgo-link-order','torrentngd',out/'torrentngd.link-order',out/'link-order.manifest.json'),('rtorrent','rtorrent',root/'rtorrent-prefix/bin/rtorrent',root/'bench-binaries/rtorrent.manifest.json')]
args=['taskset','-c','2-3',sys.executable,probe,'--repeats',3,'--size',67108864,'--rate',4194304,'--window',16,'--seed',20261027,'--order-seed',20261027,'--client-cpus','0-1','--warmup',5,'--cooldown',2,'--output',root/'paired-layout-followup']
for name,kind,binary,manifest in clients: args+=['--client',name+'='+kind+':'+str(binary),'--client-manifest',name+'='+str(manifest)]
run(args)
s=json.loads((root/'paired-layout-followup/summary.json').read_text())
a=[paired_analysis(s,'pgo-relocs',name) for name in ['pgo-bolt-4k','pgo-link-order']]
(root/'paired-layout-followup/analysis.json').write_text(json.dumps(a,indent=2)+'\n')
for result in a:
 print(result['candidate'],{k:v['paired_median'] for k,v in result['metrics'].items()},result['resource_gate'],flush=True)
