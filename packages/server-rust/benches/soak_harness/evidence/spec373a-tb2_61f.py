# SPEC-373a per-bucket table (context only; does not feed E). A copy of tb2.py
# with the 61f84658 regexes in the frozen order below: a program point is
# attributed to the FIRST bucket whose regex matches anywhere in its stack, so
# the order is part of the program.
import gzip,json,sys,re
d=json.load(gzip.open(sys.argv[1]))
ftbl=d['ftbl']; tot=sum(p['tb'] for p in d['pps']); tbk=sum(p['tbk'] for p in d['pps'])
sites=[
 ('engine get clone', r'engines/hashmap\.rs:9[1-3]:'),
 ('engine update_in_place clone', r'engines/hashmap\.rs:(13[5-7]|15[7-9]):'),
 ('engine snapshot_iter clone', r'engines/hashmap\.rs:25[4-6]:'),
 ('engine random_samples clone', r'engines/hashmap\.rs:26[7-9]:'),
 ('WB queue clone', r'write_behind\.rs:246[0-6]:'),
 ('WB staging clone', r'write_behind\.rs:251[7-9]:'),
 ('WB WAL snapshot clone', r'write_behind\.rs:242[6-9]:'),
 ('WB load staged clone', r'write_behind\.rs:264[7-9]:'),
 ('WB collect_staging clone', r'write_behind\.rs:153[6-9]:'),
 ('merkle_leaf_hash', r'map_data_store\.rs:(7[2-9]|8[0-9]|9[0-7]):'),
]
agg={}; cnt={}
for p in d['pps']:
    st=' || '.join(ftbl[i] for i in p['fs'])
    k='UNMATCHED'
    for name,rx in sites:
        if re.search(rx,st): k=name; break
    agg[k]=agg.get(k,0)+p['tb']; cnt[k]=cnt.get(k,0)+p['tbk']
print(sys.argv[1],"total %.0f MB, %d blocks, te=%s"%(tot/2**20,tbk,d['te']))
for name,_ in sites+[('UNMATCHED',None)]:
    v=agg.get(name,0)
    print("%6.1f%%  %8.1f MB  %9d blk  %s"%(100*v/tot,v/2**20,cnt.get(name,0),name))
