#!/usr/bin/env python3
"""One genuine Sepolia faucet claim, then durable cooldown rejection."""
import hashlib,json,os,subprocess,time,urllib.error,urllib.request
BASE='http://127.0.0.1:18082'
RPC='https://api.testnet.cregnet.dev/sepolia'
RECIPIENT='0x91e4dfc18bf015051676f00f23782d0770b95a7a'
TOKEN='0xbC2a72107C6765364D0A822a88A6e6C61f30EAa6'
OUT='/tmp/creg-faucet-r25-real-drip.json'
assert not os.path.exists(OUT), 'A prior claim report exists; inspect it instead of submitting again'
def read(url,body=None):
    request=urllib.request.Request(url,data=None if body is None else json.dumps(body).encode(),headers={'Content-Type':'application/json','Accept':'application/json','User-Agent':'CREG-faucet-acceptance/1.0'})
    try:
        with urllib.request.urlopen(request,timeout=90) as response: return response.status,json.load(response)
    except urllib.error.HTTPError as error:
        body=error.read().decode()
        try: return error.code,json.loads(body)
        except ValueError: return error.code,{'error':'upstream_http_error','body_excerpt':body[:200]}
def rpc(method,params):
    status,result=read(RPC,{'jsonrpc':'2.0','id':1,'method':method,'params':params})
    assert status==200 and 'error' not in result,result
    return result['result']
def balance(address):
    return int(rpc('eth_call',[{'to':TOKEN,'data':'0x70a08231'+address[2:].lower().rjust(64,'0')},'latest']),16)
def solve():
    status,challenge=read(BASE+'/api/challenge'); assert status==200,challenge
    threshold=1<<(256-challenge['difficulty'])
    for nonce in range(20000000):
        if int.from_bytes(hashlib.sha256((challenge['challenge']+str(nonce)).encode()).digest(),'big')<threshold:
            return {'address':RECIPIENT,'challenge':challenge['challenge'],'nonce':str(nonce)}
    raise RuntimeError('Real proof-of-work deadline exhausted')
assert int(rpc('eth_chainId',[]),16)==11155111
before=balance(RECIPIENT)
report={'timestamp_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'chain_id':11155111,'token':TOKEN,'recipient':RECIPIENT,'recipient_tokens_before_atomic':str(before),'status':'claim_started','expected_drip_atomic':'1000000000000000000000','mock_responses':False}
def save():
    with open(OUT,'w') as output: json.dump(report,output,indent=2)
save()
started=time.monotonic()
status,claim=read(BASE+'/api/drip',solve())
report['claim_http_status']=status; report['claim']=claim; save()
assert status==200 and claim['success'] and claim.get('token_tx_hash'),claim
print(json.dumps({'status':'real_claim_confirmed','transaction_hash':claim['token_tx_hash']}),flush=True)
subprocess.run(['sudo','docker','restart','--time','10','creg-unified-faucet-db'],check=True,stdout=subprocess.DEVNULL)
for attempt in range(15):
    ready=subprocess.run(['sudo','docker','exec','creg-unified-faucet-db','pg_isready','-U','faucet','-d','creg_faucet'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode==0
    if ready: break
    time.sleep(1)
assert ready, 'PostgreSQL did not recover'
subprocess.run(['sudo','docker','restart','--time','10','creg-faucet-public-r25'],check=True,stdout=subprocess.DEVNULL)
for attempt in range(20):
    try:
        status,health=read(BASE+'/health')
        if status==200 and health['status']=='healthy': break
    except Exception: pass
    time.sleep(0.5)
else: raise RuntimeError('Faucet did not recover after PostgreSQL restart')
status,rejection=read(BASE+'/api/drip',solve())
report['cooldown_after_database_and_faucet_restart']={'http_status':status,'response':rejection,'elapsed_seconds':time.monotonic()-started}; save()
assert status==429 and rejection['retry_after_seconds']>0,rejection
receipt=rpc('eth_getTransactionReceipt',[claim['token_tx_hash']])
assert receipt and int(receipt['status'],16)==1,receipt
after=balance(RECIPIENT)
assert after-before==10**21, 'On-chain balance does not match exactly one 1000-token claim'
logs=[log for log in receipt['logs'] if log['address'].lower()==TOKEN.lower()]
assert logs,'No token contract receipt logs'
count=subprocess.check_output(['sudo','docker','exec','creg-unified-faucet-db','psql','-U','faucet','-d','creg_faucet','-Atc',"SELECT count(*) FROM faucet_rate_limits WHERE key LIKE 'addr:%'"],text=True).strip()
report.update({'status':'passed','receipt':receipt,'recipient_tokens_after_atomic':str(after),'durable_address_count':int(count),'database_restart':'passed','faucet_restart':'passed','gas_drip_enabled':False,'public_promoted':False})
save()
print(json.dumps({'status':'passed','transaction_hash':claim['token_tx_hash'],'recipient':RECIPIENT,'amount':'1000 tCREG','cooldown_http_status':status,'database_and_faucet_restarted':True}),flush=True)
