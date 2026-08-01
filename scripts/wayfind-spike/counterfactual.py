#!/usr/bin/env python3
"""Blend counterfactual on the SAME rows: re-rank each query's shipped top-50 under alternative
weightings and report the distilled share of the resulting top-10.

STATED LIMIT: this can only re-order the 50 rows the shipped blend already surfaced. It cannot
promote a resource that never entered the top-50, so every effect reported here is a LOWER BOUND.
"""
import csv, sys
from collections import defaultdict

cls = {}
for line in open('classify_out.txt'):
    p = [x.strip() for x in line.split('|')]
    if len(p) == 5 and len(p[0]) == 36:
        try:
            cls[p[0]] = dict(table=p[1], regions=int(p[3]), chunks=int(p[4]))
        except ValueError:
            pass

def klass(rid):
    c = cls.get(rid)
    return None if not c else ('distilled' if c['table'] == 'kb_cogmaps' else 'raw')

BLENDS = [
    ('shipped        1.0*fts + 1.0*vec + 0.5*graph', 1.0, 1.0, 0.5),
    ('fts halved     0.5*fts + 1.0*vec + 0.5*graph', 0.5, 1.0, 0.5),
    ('fts dropped    0.0*fts + 1.0*vec + 0.5*graph', 0.0, 1.0, 0.5),
    ('vec only       0.0*fts + 1.0*vec + 0.0*graph', 0.0, 1.0, 0.0),
    ('graph doubled  1.0*fts + 1.0*vec + 1.0*graph', 1.0, 1.0, 1.0),
]

for path in sys.argv[1:]:
    rows = list(csv.DictReader(open(path)))
    byq = defaultdict(list)
    for r in rows:
        byq[int(r['qi'])].append(r)
    print(f"=== {path} — distilled share of top-10 under alternative blends ===")
    print(f"  {'blend':46s} {'distilled/200':>14s} {'share':>7s} {'zero-distilled queries':>23s}")
    for name, wf, wv, wg in BLENDS:
        d = n = 0
        zero = 0
        for qi, qr in sorted(byq.items()):
            scored = sorted(qr, key=lambda r: -(wf*float(r['fts']) + wv*float(r['vec'])
                                                + wg*float(r['graph'])))[:10]
            ks = [klass(r['resource_id']) for r in scored]
            d += sum(1 for k in ks if k == 'distilled')
            n += len(ks)
            if not any(k == 'distilled' for k in ks):
                zero += 1
        print(f"  {name:46s} {d:6d}/{n:<7d} {100*d/n:6.1f}% {zero:14d}/20")
    print()
