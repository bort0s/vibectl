# exclusive-hold-poll.ps1 — does anything hold a file open, and for how long?
#
# ADR-0011 §7b installs while Claude Code is running, and §2 round 3g measured
# that a rename-over is refused when another process holds the destination
# without FILE_SHARE_DELETE. Whether that matters depends on a fact about the
# other program, not about us: DOES IT HOLD settings.json OPEN?
#
# Round 3g answered a proxy — replacements during a `claude doctor` run, none
# refused — with NO REACHABILITY PREMISE. Both are short; if the read never
# overlapped a replacement, the zero was a guard never reached. This asks the
# question directly instead: attempt an EXCLUSIVE open on a fixed cadence and
# count refusals. A refusal means somebody else has it.
#
# PAIRED, and the pairing is what makes a zero reportable: run it against a file
# held on purpose and it must report refusals. Measured — 88 of 89 — so the
# instrument sees a hold when there is one.
#
# WHAT IT CANNOT SEE: a hold shorter than the poll interval. That is the
# open-and-close case rather than an extended one, and it is covered by the
# replacement measurement instead.
#
# usage: powershell -File exclusive-hold-poll.ps1 -settings <path> -marker <path> -out <path>
#        (stops when <marker> appears)
param(
  [Parameter(Mandatory=$true)][string]$settings,
  [Parameter(Mandatory=$true)][string]$marker,
  [Parameter(Mandatory=$true)][string]$out,
  [int]$intervalMs = 20
)
$attempts = 0; $exclusive = 0; $refused = 0; $first = ''
while (-not (Test-Path $marker)) {
  $attempts++
  try {
    $h = [System.IO.File]::Open($settings, 'Open', 'Read', 'None')
    $exclusive++
    $h.Close()
  } catch {
    $refused++
    if ($first -eq '') { $first = $_.Exception.Message }
  }
  Start-Sleep -Milliseconds $intervalMs
}
"attempts=$attempts exclusive=$exclusive refused=$refused first=$first" |
  Set-Content -Path $out -Encoding utf8
