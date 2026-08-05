#!/usr/bin/env python3
import sys, collections

def load(path):
    rows = []
    for line in open(path):
        line = line.strip()
        parts = [p.strip() for p in line.split('|')]
        if len(parts) != 4:            # headers, echoes, row counts
            continue
        qi, chunk, res, vis = parts
        if not qi.isdigit():
            continue
        rows.append((int(qi), chunk, res, vis == 't'))
    return rows

h, e = load('out_hnsw.txt'), load('out_exact.txt')
print(f"loaded: hnsw={len(h)} exact={len(e)}\n")

def by_q(rows):
    d = collections.defaultdict(list)
    for qi, c, r, v in rows:
        d[qi].append((c, r, v))
    return d

H, E = by_q(h), by_q(e)
queries = [l.strip() for l in open('queries.txt') if l.strip()]

print(f"{'q':>2} {'hnsw_ch':>7} {'ex_ch':>6} {'ch_recall':>9} {'hnsw_res':>8} {'ex_res':>6} "
      f"{'hnsw_adm':>8} {'ex_adm':>6}  query")
print('-'*110)
tot = collections.Counter()
per_q = []
for qi in sorted(E):
    hh, ee = H.get(qi, []), E[qi]
    hc, ec = {c for c,_,_ in hh}, {c for c,_,_ in ee}
    hr, er = {r for _,r,_ in hh}, {r for _,r,_ in ee}
    ha, ea = {r for _,r,v in hh if v}, {r for _,r,v in ee if v}
    rec = len(hc & ec) / len(ec) if ec else 0
    per_q.append((qi, len(hc), len(ec), rec, len(hr), len(er), len(ha), len(ea)))
    tot['hc'] += len(hc); tot['ec'] += len(ec)
    tot['ha'] += len(ha); tot['ea'] += len(ea)
    tot['ov'] += len(hc & ec)
    print(f"{qi:>2} {len(hc):>7} {len(ec):>6} {rec:>8.0%} {len(hr):>8} {len(er):>6} "
          f"{len(ha):>8} {len(ea):>6}  {queries[qi-1][:44]}")

n = len(per_q)
print('-'*110)
print(f"TOTAL chunks     hnsw={tot['hc']:>5}  exact={tot['ec']:>5}   chunk recall = {tot['ov']/tot['ec']:.1%}")
print(f"TOTAL admitted   hnsw={tot['ha']:>5}  exact={tot['ea']:>5}   admission ratio = {tot['ha']/tot['ea']:.1%}")
print(f"MEAN per query   hnsw chunks={tot['hc']/n:.1f}  admitted={tot['ha']/n:.1f} "
      f"| exact chunks={tot['ec']/n:.1f} admitted={tot['ea']/n:.1f}")

hcs = sorted(x[1] for x in per_q)
print(f"\nhnsw chunks/query: min={hcs[0]} median={hcs[n//2]} max={hcs[-1]}  "
      f"(ef_search=40 -> cap at 40?  over-40 queries = {sum(1 for c in hcs if c > 40)}/{n})")
adm = sorted(x[6] for x in per_q)
print(f"hnsw admitted/query: min={adm[0]} median={adm[n//2]} max={adm[-1]}")
adme = sorted(x[7] for x in per_q)
print(f"exact admitted/query: min={adme[0]} median={adme[n//2]} max={adme[-1]}")
