"""Sweep for a retracted claim across BOTH branches, whitespace-normalised.

A line-oriented grep cannot see a claim that spans a hard line wrap, and prose
in this repository is hard-wrapped at 80 columns -- so the longest statements,
the ones carrying arguments, are exactly the ones it misses. This normalises
whitespace and comment/doc prefixes before matching.

It also refuses to print a total it did not finish computing: a partial sweep
that ends in "sites: 4" is the instrument reporting a bounded answer from an
unbounded search.
"""
import re
import subprocess
import sys

NEAR = re.compile(
    r'(replac\w*\s+(?:a\s+)?file\s+(?:the\s+)?tool\s+could\s+not\s+read'
    r'|replaced\s+the\s+file\s+it\s+could\s+not\s+read'
    r'|edited\s+agent,\s+(?:replaced|gone)'
    r'|through\s+\*{0,2}one\s+arm\*{0,2},\s+so\s+it\s+replac'
    r'|constraint\s+2\s+violat'
    r'|violat\w*\s+(?:hard\s+)?constraint\s+2'
    r')', re.I)

PREFIX = re.compile(r'^[ \t]*(///|//!|//|[*#>|])?[ \t]*', re.M)
EXT = ('.md', '.rs', '.sh', '.js', '.toml', '.yml', '.yaml', '.txt')


def run(args):
    r = subprocess.run(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return r.stdout.decode('utf-8', 'replace')


def norm(text):
    return re.sub(r'\s+', ' ', PREFIX.sub(' ', text))


scanned = 0
hits = []
for ref in sys.argv[1:]:
    names = [f for f in run(['git', 'ls-tree', '-r', '--name-only', ref]).split('\n')
             if f.endswith(EXT)]
    for f in names:
        flat = norm(run(['git', 'show', '%s:%s' % (ref, f)]))
        scanned += 1
        for m in NEAR.finditer(flat):
            lo = max(0, m.start() - 170)
            hits.append('%s:%s\n    ...%s...\n' % (ref, f, flat[lo:m.end() + 130]))

for h in hits:
    print(h)
print('files scanned: %d   sites: %d' % (scanned, len(hits)))
