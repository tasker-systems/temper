#!/usr/bin/env python3
import csv, sys
from collections import defaultdict

# --- load classification ---
cls = {}
for line in open('classify_out.txt'):
    parts = [p.strip() for p in line.split('|')]
    if len(parts) == 5 and len(parts[0]) == 36 and '-' in parts[0]:
        rid, table, anchor, regions, nchunks = parts
        try:
            regions = int(regions); nchunks = int(nchunks)
        except ValueError:
            continue
        cls[rid] = dict(table=table, anchor=anchor, regions=regions, chunks=nchunks)

rows = list(csv.DictReader(open(sys.argv[1])))
LIMIT_TOP = int(sys.argv[2]) if len(sys.argv) > 2 else 10

def kind(rid):
    c = cls.get(rid)
    if not c: return ('unknown', 'unknown')
    k = 'distilled' if c['table'] == 'kb_cogmaps' else 'raw'
    arm = 'region' if c['regions'] > 0 else 'cold-start'
    return (k, arm)

print("=== A. the prior session's 4-way table, reproduced at top-%d ===" % LIMIT_TOP)
tab = defaultdict(int); tot = 0
for r in rows:
    if int(r['rank']) > LIMIT_TOP: continue
    k, arm = kind(r['resource_id']); tab[(arm, k)] += 1; tot += 1
for (arm, k), n in sorted(tab.items(), key=lambda x: -x[1]):
    print(f"  {arm:11s} {k:9s} {n:4d}  {100*n/tot:5.1f}%")
print(f"  {'TOTAL':11s} {'':9s} {tot:4d}")
draw = sum(n for (a, k), n in tab.items() if k == 'raw')
print(f"  raw share: {100*draw/tot:.1f}%   distilled share: {100*(tot-draw)/tot:.1f}%")

print()
print("=== B. distilled share by rank depth (does distilled exist further down?) ===")
for depth in (1, 3, 5, 10, 20, 30, 50):
    n = d = 0
    for r in rows:
        if int(r['rank']) > depth: continue
        k, _ = kind(r['resource_id']); n += 1; d += (k == 'distilled')
    print(f"  top-{depth:2d}: {d:4d}/{n:4d} distilled = {100*d/n:5.1f}%")

print()
print("=== C. queries returning ZERO distilled rows, by depth ===")
for depth in (10, 25, 50):
    zero = []
    for qi in sorted({int(r['qi']) for r in rows}):
        qr = [r for r in rows if int(r['qi']) == qi and int(r['rank']) <= depth]
        if not any(kind(r['resource_id'])[0] == 'distilled' for r in qr):
            zero.append(qi)
    print(f"  depth {depth:2d}: {len(zero):2d}/20 queries  {zero}")

print()
print("=== D. arm-score decomposition by class (top-%d) ===" % LIMIT_TOP)
agg = defaultdict(lambda: [0, 0.0, 0.0, 0.0, 0.0, 0])
for r in rows:
    if int(r['rank']) > LIMIT_TOP: continue
    k, _ = kind(r['resource_id'])
    a = agg[k]
    a[0] += 1
    a[1] += float(r['fts']); a[2] += float(r['vec'])
    a[3] += float(r['graph']); a[4] += float(r['combined'])
    a[5] += cls.get(r['resource_id'], {}).get('chunks', 0)
print(f"  {'class':10s} {'n':>4s} {'fts':>7s} {'vec':>7s} {'graph':>7s} {'combined':>9s} {'chunks':>7s}")
for k, a in sorted(agg.items()):
    n = a[0]
    print(f"  {k:10s} {n:4d} {a[1]/n:7.4f} {a[2]/n:7.4f} {a[3]/n:7.4f} {a[4]/n:9.4f} {a[5]/n:7.2f}")

print()
print("=== E. how tight is the race at the top-%d boundary? ===" % LIMIT_TOP)
margins = []
for qi in sorted({int(r['qi']) for r in rows}):
    qr = sorted([r for r in rows if int(r['qi']) == qi], key=lambda x: int(x['rank']))
    if len(qr) > LIMIT_TOP:
        m = float(qr[LIMIT_TOP-1]['combined']) - float(qr[LIMIT_TOP]['combined'])
        margins.append(m)
margins.sort()
print(f"  combined-score gap across the rank-{LIMIT_TOP}/{LIMIT_TOP+1} boundary, n={len(margins)}")
print(f"    min {margins[0]:.5f}  p50 {margins[len(margins)//2]:.5f}  max {margins[-1]:.5f}")
print(f"    mean {sum(margins)/len(margins):.5f}")
print(f"  (compare: best-of-N adds ~0.0125 to raw's mean vec_norm vs distilled)")

print()
print("=== F. chunk count of returned rows vs corpus baseline ===")
for k, base in (('distilled', 1.88), ('raw', 10.50)):
    got = [cls[r['resource_id']]['chunks'] for r in rows
           if int(r['rank']) <= LIMIT_TOP and kind(r['resource_id'])[0] == k
           and r['resource_id'] in cls]
    if got:
        print(f"  {k:10s} returned mean chunks {sum(got)/len(got):6.2f}  vs corpus mean {base:5.2f}")
