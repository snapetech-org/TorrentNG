#!/usr/bin/env python3
"""Private native-daemon upload trial. The receiver is a deterministic BEP 3 test peer."""
import argparse, base64, ctypes, hashlib, json, os, socket, struct, subprocess, threading, time, urllib.request
from pathlib import Path

BLOCK=16384
PIECE=1048576
TOKEN='local-native-seed-probe'

def bencode(x):
    if isinstance(x,int):return b'i'+str(x).encode()+b'e'
    if isinstance(x,bytes):return str(len(x)).encode()+b':'+x
    if isinstance(x,str):return bencode(x.encode())
    if isinstance(x,list):return b'l'+b''.join(map(bencode,x))+b'e'
    return b'd'+b''.join(bencode(k)+bencode(v) for k,v in sorted(x.items()))+b'e'

def port():
    with socket.socket() as s:s.bind(('127.0.0.1',0));return s.getsockname()[1]

def fixture(root,mib):
    root.mkdir(parents=True,exist_ok=True)
    payload=root/f'payload-{mib}.bin'; torrent=root/f'payload-{mib}.torrent'; manifest=root/f'payload-{mib}.json'
    if not manifest.exists():
        hashes=[]; sha=hashlib.sha256()
        with payload.open('xb') as f:
            for i in range(mib):
                data=hashlib.shake_256(b'TorrentNG-native-followup-v1'+struct.pack('>I',i)).digest(PIECE)
                f.write(data);hashes.append(hashlib.sha1(data, usedforsecurity=False).digest());sha.update(data)  # nosemgrep: python.lang.security.insecure-hash-algorithms.insecure-hash-algorithm-sha1 -- BitTorrent v1 piece hashes require SHA-1.
        info={b'length':mib*PIECE,b'name':payload.name.encode(),b'piece length':PIECE,b'pieces':b''.join(hashes),b'private':1}
        torrent.write_bytes(bencode({b'info':info}))
        info_hash=hashlib.sha1(bencode(info), usedforsecurity=False).hexdigest()  # nosemgrep: python.lang.security.insecure-hash-algorithms.insecure-hash-algorithm-sha1 -- BitTorrent v1 info hashes require SHA-1.
        manifest.write_text(json.dumps({'size':mib*PIECE,'sha256':sha.hexdigest(),'info_hash':info_hash}))
    m=json.loads(manifest.read_text())
    assert payload.stat().st_size==m['size']
    with payload.open('rb') as f:assert hashlib.file_digest(f,'sha256').hexdigest()==m['sha256']
    return payload,torrent,m

class Timespec(ctypes.Structure):_fields_=[('tv_sec',ctypes.c_long),('tv_nsec',ctypes.c_long)]
libc=ctypes.CDLL(None,use_errno=True)
def cpu_seconds(pid):
    clock=ctypes.c_int(); ts=Timespec()
    if libc.clock_getcpuclockid(pid,ctypes.byref(clock))!=0:raise RuntimeError('process clock unavailable')
    if libc.clock_gettime(clock.value,ctypes.byref(ts))!=0:raise OSError(ctypes.get_errno())
    return ts.tv_sec+ts.tv_nsec/1e9

def memory(pid):
    result={}
    for line in Path(f'/proc/{pid}/status').read_text().splitlines():
        key,_,value=line.partition(':')
        if key in ('VmRSS','RssAnon','RssFile','VmHWM','Threads'):result[key]=int(value.split()[0])
    return result

def thread_cpu(pid):
    result={}
    for p in Path(f'/proc/{pid}/task').iterdir():
        try:
            line=(p/'stat').read_text();fields=line[line.rfind(')')+2:].split()
            result[p.name]={'name':(p/'comm').read_text().strip(),'cpu_s':(int(fields[11])+int(fields[12]))/os.sysconf('SC_CLK_TCK'),'user_s':int(fields[11])/os.sysconf('SC_CLK_TCK'),'system_s':int(fields[12])/os.sysconf('SC_CLK_TCK')}
        except FileNotFoundError:pass
    return result

def procfs_child_pid(local_pid):
    # The tool host mounts the outer /proc inside a PID namespace. CPU clocks
    # use namespace PIDs; /proc paths must use the matching outer child PID.
    status=Path('/proc/self/status').read_text()
    outer_self=int(next(l.split()[1] for l in status.splitlines() if l.startswith('Tgid:')))
    for child in Path('/proc').glob('[0-9]*'):
        try:child_status=(child/'status').read_text()
        except (FileNotFoundError,ProcessLookupError):continue
        parent=int(next(l.split()[1] for l in child_status.splitlines() if l.startswith('PPid:')))
        if parent!=outer_self:continue
        ids=next(l.split()[1:] for l in child_status.splitlines() if l.startswith('NSpid:'))
        if int(ids[-1])==local_pid:return int(child.name)
    raise RuntimeError('cannot reconcile child PID with procfs namespace')

def exact(s,n):
    data=bytearray()
    while len(data)<n:
        part=s.recv(n-len(data))
        if not part:raise EOFError('peer closed before completion')
        data.extend(part)
    return bytes(data)

def frame(s):
    n=struct.unpack('>I',exact(s,4))[0]
    if n>2*PIECE:raise RuntimeError('oversized frame')
    return exact(s,n)

def main():
    ap=argparse.ArgumentParser();ap.add_argument('--binary',type=Path,required=True);ap.add_argument('--output',type=Path,required=True)
    ap.add_argument('--fixture-dir',type=Path,required=True);ap.add_argument('--mib',type=int,default=256);ap.add_argument('--rate-mib',type=float,default=4)
    ap.add_argument('--window',type=int,default=32);ap.add_argument('--refill',type=int,default=4);ap.add_argument('--diagnostics',action='store_true');ap.add_argument('--label',default='baseline');ap.add_argument('--idle-seconds',type=float,default=0)
    a=ap.parse_args(); out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
    if a.mib<1 or not (1<=a.refill<=a.window) or a.rate_mib<0 or a.idle_seconds<0:
        raise ValueError('require positive size, 1 <= refill <= window, and nonnegative rate/idle')
    payload,torrent,meta=fixture(a.fixture_dir.resolve(),a.mib)
    api,peer=port(),port(); session=out/'session';session.mkdir()
    config=out/'config.toml'
    config.write_text(f'''[daemon]
api_bind = "127.0.0.1:{api}"
session_dir = {json.dumps(str(session))}
log_level = "warn"
shutdown_timeout_secs = 2
[network]
listen_port = {peer}
max_peers = 32
upload_rate_limit = 33554432
download_rate_limit = 33554432
[storage]
download_dir = {json.dumps(str(payload.parent))}
[dht]
enabled = false
bootstrap_nodes = []
[tracker]
allow_loopback_egress = true
[auth]
api_tokens = ["{TOKEN}"]
[logging]
format = "json"
profile = "basic"
filter = "{'warn,torrentng::upload_batches=debug' if a.diagnostics else 'warn'}"
''')
    env=os.environ.copy();env.update(TORRENTNGD_CONFIG=str(config),TOKIO_WORKER_THREADS='2',TNG_STORAGE_BACKEND='pread',RUST_LOG='warn,torrentng::upload_batches=debug' if a.diagnostics else 'warn')
    log=(out/'daemon.log').open('w');proc=subprocess.Popen([str(a.binary.resolve())],env=env,stdout=log,stderr=log)
    def api_call(path,body=None):
        request=urllib.request.Request(f'http://127.0.0.1:{api}'+path,data=json.dumps(body).encode() if body is not None else None,headers={'Authorization':'Bearer '+TOKEN,'Content-Type':'application/json'})
        with urllib.request.urlopen(request,timeout=5) as r:return r.read()  # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected -- The URL is fixed to the test daemon on loopback.
    stop=threading.Event(); samples=[]; receiver=out/'received.bin'
    try:
        procfs_pid=procfs_child_pid(proc.pid)
        for _ in range(200):
            if proc.poll() is not None:raise RuntimeError('daemon exited during startup')
            try:api_call('/health');break
            except (OSError,urllib.error.URLError):time.sleep(.1)
        else:raise TimeoutError('daemon readiness')
        response=json.loads(api_call('/api/v1/torrents',{'torrent_b64':base64.b64encode(torrent.read_bytes()).decode(),'save_path':str(payload.parent),'start':True}))
        assert response['info_hash']==meta['info_hash'],response
        for _ in range(600):
            rows=json.loads(api_call('/api/qb/v2/torrents/info?hashes='+meta['info_hash']))
            if rows and rows[0]['progress']>=1:break
            time.sleep(.1)
        else:raise TimeoutError('native daemon did not recognize verified seed')
        info_hash=bytes.fromhex(meta['info_hash'])
        listener=socket.socket();listener.bind(('127.0.0.1',0));listener.listen(1);listener.settimeout(20)
        api_call('/api/v1/torrents/'+meta['info_hash']+'/peers',{'peers':['127.0.0.1:'+str(listener.getsockname()[1])]})
        s,_=listener.accept();listener.close()
        with s, receiver.open('w+b') as f:
            s.settimeout(30);s.setsockopt(socket.IPPROTO_TCP,socket.TCP_NODELAY,1)
            s.sendall(b'\x13BitTorrent protocol'+b'\0'*8+info_hash+b'-AU0001-123456789012')
            handshake=exact(s,68);assert handshake[28:48]==info_hash
            s.sendall(struct.pack('>IB',1,2))
            while True:
                msg=frame(s)
                if msg and msg[0]==1:break
            if a.idle_seconds:
                idle_threads=thread_cpu(procfs_pid);idle_cpu=cpu_seconds(proc.pid);time.sleep(a.idle_seconds)
                idle_result={'seconds':a.idle_seconds,'cpu_seconds':cpu_seconds(proc.pid)-idle_cpu,'before_threads':idle_threads,'after_threads':thread_cpu(procfs_pid)}
                (out/'idle.json').write_text(json.dumps(idle_result,indent=2)+'\n');print(json.dumps(idle_result),flush=True)
            total=meta['size']//BLOCK;sent=0;seen=set();received=0
            def requests(count):
                nonlocal sent
                buffer=bytearray()
                for _ in range(min(count,total-sent)):
                    offset=sent*BLOCK;buffer.extend(struct.pack('>IBIII',13,6,offset//PIECE,offset%PIECE,BLOCK));sent+=1
                if buffer:s.sendall(buffer)
            def sample():
                while not stop.is_set():
                    try:samples.append({'t':time.monotonic(),**memory(procfs_pid)})
                    except FileNotFoundError:break
                    stop.wait(.25)
            sampler=threading.Thread(target=sample,daemon=True);sampler.start()
            before_threads=thread_cpu(procfs_pid);before=memory(procfs_pid);cpu0=cpu_seconds(proc.pid);start=time.monotonic();requests(a.window)
            while received<total:
                msg=frame(s)
                if not msg:continue
                if msg[0]==0:raise RuntimeError('seed choked during controlled transfer')
                if msg[0]!=7:continue
                p,b=struct.unpack('>II',msg[1:9]);data=msg[9:];offset=p*PIECE+b;index=offset//BLOCK
                if len(data)!=BLOCK or offset%BLOCK or index>=sent or index in seen:raise RuntimeError('invalid or duplicate Piece')
                seen.add(index);os.pwrite(f.fileno(),data,offset);received+=1
                if a.rate_mib:
                    delay=(received*BLOCK)/(a.rate_mib*PIECE)-(time.monotonic()-start)
                    if delay>0:time.sleep(delay)
                if received%a.refill==0:requests(a.refill)
            elapsed=time.monotonic()-start;cpu=cpu_seconds(proc.pid)-cpu0;after=memory(procfs_pid);after_threads=thread_cpu(procfs_pid)
            stop.set();sampler.join()
            f.seek(0);actual=hashlib.file_digest(f,'sha256').hexdigest();assert actual==meta['sha256']
            # The first stats response is deliberately a fast partial snapshot;
            # let its asynchronous collector finish outside the timed window.
            for _ in range(30):
                metrics=api_call('/metrics').decode()
                values=[int(line.split()[-1]) for line in metrics.splitlines() if line.startswith('torrentng_storage_bytes_read_by_class_total{class="peer_read"}')]
                if values and values[0]>=meta['size']:break
                time.sleep(.1)
            (out/'metrics.txt').write_text(metrics)
        result={'label':a.label,'binary_sha256':hashlib.sha256(a.binary.read_bytes()).hexdigest(),'payload_sha256':actual,'payload_bytes':meta['size'],'cpu_seconds':cpu,'cpu_s_per_gib':cpu/(meta['size']/2**30),'elapsed_seconds':elapsed,'rate_cap_mib_s':a.rate_mib,'request_window':a.window,'refill_blocks':a.refill,'diagnostics':a.diagnostics,'before':before,'after':after,'peak_rss_kib':max(x['VmRSS'] for x in samples),'peak_anon_kib':max(x['RssAnon'] for x in samples),'samples':samples}
        result.update(before_threads=before_threads,after_threads=after_threads)
        (out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
        print(json.dumps({k:v for k,v in result.items() if k!='samples'}),flush=True)
        receiver.unlink()
    finally:
        stop.set()
        if proc.poll() is None:
            proc.terminate()
            try:proc.wait(timeout=10)
            except subprocess.TimeoutExpired:proc.kill();proc.wait()
        log.close()

if __name__=='__main__':main()
