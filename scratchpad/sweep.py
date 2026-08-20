"""Sweep a repository's text for a claim, across refs, without a line ceiling.

WHY THIS EXISTS, AND WHY IT IS COMMITTED RATHER THAN DESCRIBED
--------------------------------------------------------------
ADR-0002 §7: a retraction is finished when the residue is swept, not when the
retraction is written. That rule has failed three times in this repository, and
the third failure was not a lapse -- the sweep was run, three times, and still
missed ten of thirteen occurrences.

The cause is structural. Prose here is HARD-WRAPPED AT 80 COLUMNS, and `grep`
matches within a line. So a line-oriented search cannot see any statement
longer than one line, which selects against exactly the sentences that carry
arguments: a claim short enough to fit on one line is rarely the claim worth
retracting. Every "searched, zero occurrences" over prose in this project's
history has that ceiling and did not declare it.

So the ceiling is the thing to remove, and this removes it:

  * comment and doc prefixes (`///`, `//!`, `//`, `#`, `*`, `>`, `|`) stripped,
    so a claim reads the same in an ADR and in a doc comment;
  * all whitespace collapsed, so a hard wrap is invisible to the match;
  * run over REFS, not the working tree, so every branch that will merge is
    covered rather than whichever one happens to be checked out;
  * source included, not only documents -- the highest-cost residue found so
    far was a prose comment in shipped code, at the call site of the repair;
  * FILES SCANNED PRINTED BESIDE SITES FOUND, so a bounded answer is
    distinguishable from a search that stopped early. Its own first version
    died mid-run on an encoding error and still printed a total.

USAGE
    python scratchpad/sweep.py <ref> [<ref> ...] [--pattern REGEX]

With no --pattern it sweeps for the 2026-08-19 `agents` claim, withdrawn
2026-08-20 (ADR-0001 §3b), which is the case it was built for.
"""
import io
import re
import subprocess
import sys

# The instrument must not die on its own output. An earlier version raised
# UnicodeEncodeError while PRINTING a match that contained an arrow, on a
# console whose default codec is cp1252 -- a failure in the channel between the
# instrument and its reader, which is the class this repository catalogues. It
# had already found the sites; it just could not say so.
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding='utf-8', errors='replace')

DEFAULT = (
    r'replac\w*\s+(?:a\s+)?file\s+(?:the\s+)?tool\s+could\s+not\s+read'
    r'|replaced\s+the\s+file\s+it\s+could\s+not\s+read'
    r'|edited\s+agent,\s+(?:replaced|gone)'
    r'|through\s+\*{0,2}one\s+arm\*{0,2},\s+so\s+it\s+replac'
    r'|constraint\s+2\s+violat'
    r'|violat\w*\s+(?:hard\s+)?constraint\s+2'
)

PREFIX = re.compile(r'^[ \t]*(///|//!|//|[*#>|])?[ \t]*', re.M)
EXT = ('.md', '.rs', '.sh', '.js', '.ps1', '.toml', '.yml', '.yaml', '.txt')


def run(args):
    r = subprocess.run(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return r.stdout.decode('utf-8', 'replace')


def flatten(text):
    """A hard wrap must not hide a claim, and a doc prefix must not either."""
    return re.sub(r'\s+', ' ', PREFIX.sub(' ', text))


def main(argv):
    pattern = DEFAULT
    refs = []
    i = 0
    while i < len(argv):
        if argv[i] == '--pattern':
            pattern = argv[i + 1]
            i += 2
        else:
            refs.append(argv[i])
            i += 1
    if not refs:
        print(__doc__)
        return 2

    needle = re.compile('(%s)' % pattern, re.I)
    scanned, hits = 0, []
    for ref in refs:
        names = [f for f in run(['git', 'ls-tree', '-r', '--name-only', ref]).split('\n')
                 if f.endswith(EXT)]
        if not names:
            print('%s: no files -- is that a ref?' % ref)
            return 2
        for f in names:
            flat = flatten(run(['git', 'show', '%s:%s' % (ref, f)]))
            scanned += 1
            for m in needle.finditer(flat):
                lo = max(0, m.start() - 170)
                hits.append('%s:%s\n    ...%s...\n' % (ref, f, flat[lo:m.end() + 130]))

    # **BUILT WHOLE, THEN EMITTED IN ONE ACT.** An earlier version printed each
    # match as it went and died partway through on `UnicodeEncodeError` -- it
    # had already found every site, and the failure was in the channel to the
    # reader. Printing incrementally means such a death leaves a SUBSET on
    # screen under a traceback that reads like a formatting nuisance: the
    # numbers shown are real, they are just not all of them, and nothing says
    # so. An instrument that dies after measuring and before reporting has not
    # produced a result, so the report is assembled first and written once.
    #
    # The trailing marker is what makes truncation visible if the write itself
    # is cut off -- a full disk, a closed console, a broken pipe.
    report = ''.join(hits)
    # Both numbers, always. A site count with no denominator is the same
    # unbounded-answer problem one level up.
    report += 'files scanned: %d   sites: %d\n-- end of sweep --\n' % (scanned, len(hits))
    sys.stdout.write(report)
    sys.stdout.flush()
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
