#!/usr/bin/env python3
"""Restart only O1/Prometheus and compare an anchored real TSDB query."""
import json, subprocess, time, urllib.parse, urllib.request

OUT='/tmp/creg-metrics-restart-retention.json'
PROM='http://127.0.0.1:9090'
NODE='http://127.0.0.1:18080'
CHAIN='creg-phase-c-testnet-2026-09-26-r7'
GENESIS='1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234'
end=int(time.time())-120
query='creg_chain_tip_height{chain_id="'+CHAIN+'",network_genesis_hash="'+GENESIS+'",metrics_source="explorer-primary"}'
params=urllib.parse.urlencode({'query':query,'start':end-900,'end':end,'step':15})
def get(url):
    with urllib.request.urlopen(url,timeout=20) as response: return json.load(response)
def saved(): return get(PROM+'/api/v1/query_range?'+params)['data']['result']
def docker(*args): return subprocess.check_output(['sudo','docker',*args],text=True).strip()
def wait_ready(url):
    deadline=time.time()+60
    while time.time()<deadline:
        try:
            with urllib.request.urlopen(url,timeout=5) as response:
                if response.status==200: return
        except Exception: pass
        time.sleep(1)
    raise RuntimeError('Readiness deadline exceeded: '+url)
before=saved()
assert len(before)==1 and len(before[0]['values'])>20, 'Missing real retained samples'
volume_before=json.loads(docker('inspect','creg-gcp-prometheus','--format','{{json .Mounts}}'))
identity_before=get(NODE+'/v1/public/chain/stats')
report={'timestamp_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'query':query,'fixed_range':{'start':end-900,'end':end,'step':15},'before':before,'status':'started','restart_targets':['creg-gcp-prometheus','creg-phase-c-r7-o1'],'validator_interrupted':False}
def save():
    with open(OUT,'w') as output: json.dump(report,output,indent=2)
save()
docker('restart','--time','20','creg-gcp-prometheus')
wait_ready(PROM+'/-/ready')
after_collector=saved()
assert after_collector==before, 'Prometheus lost historical values on restart'
report['collector_restart']='passed'; save()
docker('restart','--time','20','creg-phase-c-r7-o1')
wait_ready(NODE+'/v1/public/health')
after_observer=saved()
assert after_observer==before, 'Stored history changed during observer restart'
volume_after=json.loads(docker('inspect','creg-gcp-prometheus','--format','{{json .Mounts}}'))
assert volume_after==volume_before, 'Collector persistent mounts changed'
stats=get(NODE+'/v1/public/chain/stats')
for field in ['chain_id','tip_height','tip_hash','finalized_height','genesis_hash']:
    assert stats[field]==identity_before[field], field
history=get(NODE+'/v1/public/metrics/history?range=1h')
assert history['chain_id']==CHAIN and history['queried_series']==7 and history['sample_count']>30
assert any(s['timestamp_ms']<end*1000 for s in history['samples']), 'API lost pre-restart history'
report.update({'status':'passed','observer_restart':'passed','after_collector':after_collector,'after_observer':after_observer,'persistent_mounts':volume_after,'api_sample_count':history['sample_count'],'api_queried_series':history['queried_series'],'tip_height':stats['tip_height'],'tip_hash':stats['tip_hash']})
save()
print(json.dumps({k:v for k,v in report.items() if k not in ['before','after_collector','after_observer','persistent_mounts']}))
