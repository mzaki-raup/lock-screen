<#
.SYNOPSIS
  Bring commits made on the GitHub mirror back to the Forgejo.

.DESCRIPTION
  The two forges hold one history. The Forgejo is `origin` and is pushed to
  both (see CLAUDE.md, "Two forges"), so the forward direction needs nothing.
  This script is the reverse one: a web edit or a merged pull request on
  GitHub, which the Forgejo cannot know about on its own.

  Fast-forward only, deliberately. If the Forgejo also moved, this stops and
  says so rather than making a merge commit nobody reviewed; reconcile by hand
  (rebase or merge) and push. A plain `git push` to both remotes is what makes
  the two writable sides safe: a side that is behind is *rejected*, never
  overwritten -- which is also why there is no push mirror. A Forgejo push
  mirror pushes with force and would silently discard a GitHub commit the
  moment anything was pushed on the Forgejo.

.PARAMETER Branch
  The branch to bring across. Defaults to main.

.EXAMPLE
  .\scripts\sync-from-github.ps1
#>
[CmdletBinding()]
param(
    [string] $Branch = 'main'
)

$ErrorActionPreference = 'Stop'

function Invoke-Git {
    & git @args
    if ($LASTEXITCODE -ne 0) { throw "git $($args -join ' ') failed with exit code $LASTEXITCODE" }
}

$root = git rev-parse --show-toplevel 2>$null
if (-not $root) { throw 'not inside a git repository' }
Set-Location $root

if (-not (git remote | Where-Object { $_ -eq 'github' })) {
    throw "no remote named 'github'. Add it with: git remote add github https://github.com/<user>/lock-screen.git"
}

$current = (git rev-parse --abbrev-ref HEAD).Trim()
if ($current -ne $Branch) { throw "on '$current', not '$Branch'; switch first so the fast-forward lands where you expect" }
if (git status --porcelain) { throw 'the working tree is not clean; commit or stash first' }

Write-Host "fetching github/$Branch and its tags"
Invoke-Git fetch github --tags

$behind = (git rev-list --count "$Branch..github/$Branch").Trim()
$ahead = (git rev-list --count "github/$Branch..$Branch").Trim()
if ($behind -eq '0') {
    Write-Host "nothing new on GitHub ($ahead commit(s) here not yet there)"
    exit 0
}

Write-Host "GitHub has $behind commit(s) this clone does not:"
git log --oneline "$Branch..github/$Branch"

# --ff-only: a merge commit here would be one nobody reviewed on either forge.
Invoke-Git merge --ff-only "github/$Branch"

# `origin` pushes to both forges, so this also puts GitHub's own commits back
# where they came from -- a no-op there, and the Forgejo catches up.
Write-Host "pushing $Branch to both forges"
Invoke-Git push origin $Branch
