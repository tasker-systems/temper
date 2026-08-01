#!/usr/bin/env python3
"""Parse the concatenated JSON records written by run_queries.sh (payload is pretty-printed,
so the file is a stream of JSON values, not line-delimited)."""
import json, sys

def records(path):
    text = open(path).read()
    dec = json.JSONDecoder()
    i = 0
    out = []
    while i < len(text):
        while i < len(text) and text[i] in ' \n\r\t':
            i += 1
        if i >= len(text):
            break
        obj, j = dec.raw_decode(text, i)
        out.append(obj)
        i = j
    return out

if __name__ == '__main__':
    recs = records(sys.argv[1])
    mode = sys.argv[2] if len(sys.argv) > 2 else 'ids'
    if mode == 'ids':
        ids = set()
        for r in recs:
            for row in r['payload'].get('results', []):
                ids.add(row['resource_id'])
        for x in sorted(ids):
            print(x)
    elif mode == 'rows':
        w = csv = None
        import csv as _csv
        w = _csv.writer(sys.stdout)
        w.writerow(['qi', 'query', 'rank', 'resource_id', 'doc_type', 'context',
                    'fts', 'vec', 'graph', 'combined'])
        for r in recs:
            for k, row in enumerate(r['payload'].get('results', []), start=1):
                w.writerow([r['qi'], r['query'], k, row['resource_id'], row.get('doc_type'),
                            row.get('context'), row['fts_score'], row['vector_score'],
                            row['graph_score'], row['combined_score']])
    elif mode == 'diag':
        for r in recs:
            d = r['payload'].get('diagnostics', {})
            print(r['qi'], json.dumps(d))
