# Overmind's installer for Windows (ADR-0045).
#
#   irm https://overmind.eliacinti.dev/install.ps1 | iex
#
# Or, if you would rather read it first — and you should, it is somebody
# else's script:
#
#   irm https://github.com/EliaCinti/overmind/releases/latest/download/install.ps1 -OutFile install.ps1
#   Get-Content install.ps1 | more
#   .\install.ps1
#
# Windows runs Overmind through Docker Desktop with the WSL2 backend; there is
# no native Windows build, on purpose (the image is the Windows path, and it is
# what CI tests). This script never installs Docker for you.
$ErrorActionPreference = 'Stop'
# PowerShell 7.4+ turns a non-zero exit from a NATIVE command into a
# terminating error by default. Every `docker ...` call below is followed by a
# check of $LASTEXITCODE precisely so it can print a sentence a person can act
# on -- "Docker is installed but not running", not a raw error record. Without
# this, those branches are unreachable on the newer shell and reachable on 5.1,
# which is the sort of difference nobody notices until somebody is stuck.
$PSNativeCommandUseErrorActionPreference = $false

# Stamped by the release workflow: a script from release X fetches release X's
# compose file and checks it against the digest that release published.
$Tag = '__OVERMIND_TAG__'
$ComposeSha256 = '__OVERMIND_COMPOSE_SHA256__'

function Main {
    # Stamped, or nothing. The guard tests the SHAPE of the values, never the
    # sentinel: the release step rewrites EVERY occurrence, so a guard written
    # as a comparison against that sentinel becomes a value compared with
    # itself - false, and the digest check skipped. That shipped in the first
    # draft and would have published an installer that verified nothing.
    if ($Tag -notmatch '^v[0-9]') {
        Fail "This installer was never stamped with a release. Take it from a release, not from the repository: https://github.com/EliaCinti/overmind/releases/latest"
    }
    if ($ComposeSha256 -notmatch '^[0-9a-fA-F]{64}$') {
        Fail "This installer carries no digest for the compose file, so it cannot check what it downloads. Take it from a release."
    }

    Write-Host "Overmind $Tag"

    # 1 — what we are on.
    if (-not [Environment]::Is64BitOperatingSystem) {
        Fail "The published image is built for 64-bit machines only."
    }

    # 2 — ask the engine, not the filesystem.
    if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
        Fail @"
Docker is not installed. Install Docker Desktop and let it set up the WSL2
backend, open it once, then run this installer again:
    https://www.docker.com/products/docker-desktop/
"@
    }
    docker info *> $null
    if ($LASTEXITCODE -ne 0) {
        Fail @"
Docker is installed but not running. Open Docker Desktop, wait for it to say
it is ready, then run this installer again.
"@
    }
    docker compose version *> $null
    if ($LASTEXITCODE -ne 0) {
        Fail "This Docker has no 'compose' subcommand. Update Docker Desktop, then run this installer again."
    }

    # 3 — the directory, and the identity that makes it an instance of its own.
    #
    #     It lands beside whoever ran the command, in .\overmind, because that
    #     is where somebody goes looking for what a command just made — a
    #     subfolder rather than the bare working directory, which would scatter
    #     a compose file, a data\ and an agent\ into whatever you were in.
    $dir = if ($env:OVERMIND_HOME) { $env:OVERMIND_HOME } else { Join-Path (Get-Location).Path 'overmind' }
    try {
        New-Item -ItemType Directory -Force -Path $dir -ErrorAction Stop | Out-Null
    } catch {
        Fail "Cannot create $dir - this directory is not writable by you. Run it somewhere you own, or set `$env:OVERMIND_HOME first."
    }
    Set-Location $dir
    # Absolute from here on: the identity derives from it, and every line
    # printed below names it back to the person.
    $dir = (Get-Location).Path
    if (Test-Path (Join-Path $dir 'data\overmind.sqlite')) {
        Write-Host "There is already an Overmind in $dir - updating it in place."
    } else {
        $dir = Show-Elsewhere $dir
        Set-Location $dir
    }
    Write-Identity

    # 4 — the compose file, from this script's own release, checked.
    $url = "https://github.com/EliaCinti/overmind/releases/download/$Tag/docker-compose.yml"
    Write-Host "Fetching the compose file for $Tag..."
    Invoke-WebRequest -Uri $url -OutFile 'docker-compose.yml.new' -UseBasicParsing
    $got = (Get-FileHash 'docker-compose.yml.new' -Algorithm SHA256).Hash.ToLower()
    if ($got -ne $ComposeSha256.ToLower()) {
        Remove-Item 'docker-compose.yml.new' -Force
        Fail @"
The compose file does not match the digest this installer carries.
    expected  $ComposeSha256
    got       $got
Nothing was installed.
"@
    }
    Move-Item 'docker-compose.yml.new' 'docker-compose.yml' -Force

    # 5 — start it. The same line is the update.
    Write-Host "Starting Overmind..."
    docker compose up -d --pull always
    if ($LASTEXITCODE -ne 0) { Fail "Docker could not start Overmind. The output above says why." }

    # 6 — wait, then the URL and the code, read from the file the server wrote.
    $port = Get-InstancePort
    for ($i = 0; $i -lt 90; $i++) {
        try {
            Invoke-WebRequest -Uri "http://127.0.0.1:$port/" -UseBasicParsing -TimeoutSec 2 | Out-Null
            break
        } catch { Start-Sleep -Seconds 1 }
    }

    Write-Host ""
    Write-Host "Overmind is running at http://localhost:$port"
    $codeFile = Join-Path $dir 'data\setup-code'
    if (Test-Path $codeFile) {
        $code = $null
        try { $code = (Get-Content $codeFile -Raw -ErrorAction Stop).Trim() } catch { }
        Write-Host ""
        if ($code) {
            Write-Host "  The first claim costs this code, and it is asked for once:"
            Write-Host ""
            Write-Host "      $code"
        } else {
            Write-Host "  The first claim costs a code, and this shell cannot read the file:"
            Write-Host ""
            Write-Host "      $codeFile"
        }
        Write-Host ""
    } elseif (Test-Path (Join-Path $dir 'data\overmind.sqlite')) {
        Write-Host ""
        Write-Host "  Already claimed - no setup code to give."
        Write-Host ""
    } else {
        Write-Host ""
        Write-Host "  No setup code was written. The server log says why: docker compose logs overmind"
        Write-Host ""
    }

    # 7 — the three things that come next.
    Write-Host @"
  Next:
    - Open http://localhost:$port, create the owner account, paste that code.
    - Give the agent a way to pay - sign in with your Claude subscription from
      the notice above the first screen, or set ANTHROPIC_API_KEY before
      starting. (This installer cannot ask: piped to iex, it has no prompt.)
    - Your instance lives in $dir - the compose file, .\data and .\agent.
      Moving that folder moves everything. 'docker compose down -v' cannot
      touch it. The way out is an archive: the Archive button, owner only.

  To update, from ${dir}:
      docker compose up -d --pull always
"@
}

function Fail([string]$Message) {
    Write-Host ""
    Write-Error $Message
    exit 1
}

# Every Overmind this engine knows of, running or stopped, wherever it lives
# and whatever its folder is called. Asked of Docker rather than guessed at by
# scanning paths: the /data bind mount names the install directory exactly, so
# an instance whose folder was renamed is still found, and a folder merely
# CALLED overmind with nothing in it is correctly not one.
function Get-KnownInstances {
    $found = @()
    $ids = @(docker ps -a --format '{{.ID}} {{.Image}}' 2>$null |
             Where-Object { $_ -match 'overmind' } |
             ForEach-Object { ($_ -split ' ')[0] })
    foreach ($id in $ids) {
        $src = docker inspect --format '{{range .Mounts}}{{if eq .Destination "/data"}}{{.Source}}{{end}}{{end}}' $id 2>$null
        if ($src) { $found += (Split-Path $src -Parent) }
    }
    return ($found | Sort-Object -Unique)
}

# Installing beside an Overmind that already exists is safe — separate
# project, container and port — but it is rarely what somebody meant, and
# finding out later is expensive. Said before it happens, with the choice
# offered rather than assumed. Where there is no console to ask on, the safe
# thing happens: the new instance nobody has to undo.
function Show-Elsewhere([string]$dir) {
    $others = @(Get-KnownInstances | Where-Object { $_ -ne $dir })
    if ($others.Count -eq 0) { return $dir }

    Write-Host ""
    Write-Host "  Overmind is already installed on this machine:"
    Write-Host ""
    foreach ($o in $others) { Write-Host "      $o" }
    Write-Host ""
    Write-Host "  Installing here makes a SECOND, separate instance: its own data, its own"
    Write-Host "  container, its own port. Nothing above is touched."
    Write-Host ""

    if (($others.Count -ne 1) -or [Console]::IsOutputRedirected) {
        Write-Host "  Installing here. To update one of those instead, run this from its folder."
        return $dir
    }
    $answer = Read-Host "    [u] update that one instead   [n] new instance here   [q] quit"
    switch -Regex ($answer) {
        '^[uU]' {
            Write-Host "Updating the Overmind in $($others[0])."
            return $others[0]
        }
        '^[qQ]' { Fail "Nothing was installed." }
        default {
            Write-Host "Installing a second instance in $dir."
            return $dir
        }
    }
}

# What makes THIS folder an instance rather than a name collision.
#
# Compose names a project after its directory, and every default install is a
# directory called 'overmind' - so two of them would be one project: one set
# of containers, one published port, and an 'up' in the second recreating the
# first onto this folder's empty .\data. The identity comes from the whole
# path instead, written where Compose reads it.
#
# Written once and never rewritten: re-running here must not rename a project
# out from under containers already running under the old name.
function Write-Identity {
    if (Test-Path '.env') { return }
    if (Test-Path 'data\overmind.sqlite') {
        # Older than this file. Compose has been naming its containers after
        # the directory all along, so keep exactly that - inventing a new name
        # here would orphan a container that is running right now.
        $project = ((Split-Path $dir -Leaf).ToLower() -replace '[^a-z0-9_-]', '')
        if (-not $project) { $project = 'overmind' }
        $port = Get-ComposePort
    } else {
        $project = "overmind-$(Get-PathSlug)"
        $port = Get-FreePort
    }
    @"
# Written by the installer, and read by every 'docker compose' you run here.
# It is what makes this folder its own instance: Compose names a project after
# its directory, and every default install is a directory called 'overmind'.
# Two of them without this file would be ONE project - one set of containers,
# one port - and starting the second would recreate the first onto this
# folder's empty .\data.
#
# Moving this folder is still how you move the instance. Delete this file only
# if you also mean to give the instance a different name.
COMPOSE_PROJECT_NAME=$project
OVERMIND_NAME=$project
OVERMIND_PORT=$port
"@ | Set-Content -Path '.env' -Encoding ascii
}

# Eight hex of the folder's full path: stable across runs, different for every
# folder, short enough to read in 'docker ps'.
function Get-PathSlug {
    $sha = [System.Security.Cryptography.SHA256]::Create()
    $bytes = $sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($dir))
    return (($bytes | ForEach-Object { $_.ToString('x2') }) -join '').Substring(0, 8)
}

# Is somebody already on this port?
function Test-PortTaken([int]$p) {
    try {
        $c = New-Object System.Net.Sockets.TcpClient
        $ok = $c.ConnectAsync('127.0.0.1', $p).Wait(500)
        $c.Close()
        if ($ok) { return $true }
    } catch { }
    return $false
}

# 7070 when it is free - the address every document names. Otherwise a port
# belonging to THIS folder, derived from its path rather than from whatever
# happens to be listening at this second: "the first free one" hands the same
# number to two installs made while both were stopped.
function Get-FreePort {
    if (-not (Test-PortTaken 7070)) { return 7070 }
    $n = 0
    foreach ($b in [System.Text.Encoding]::UTF8.GetBytes($dir)) { $n = ($n * 31 + $b) % 100000 }
    $p = 7071 + ($n % 29)
    for ($i = 0; $i -lt 29; $i++) {
        if (-not (Test-PortTaken $p)) { return $p }
        $p = 7071 + (($p - 7070) % 29)
    }
    return 7070
}

# This instance's port: what the identity says, falling back to the compose
# file for an install that predates .env or a file edited by hand.
function Get-InstancePort {
    if (Test-Path '.env') {
        $line = Select-String -Path '.env' -Pattern '^OVERMIND_PORT=(\d+)' | Select-Object -First 1
        if ($line) { return [int]$line.Matches[0].Groups[1].Value }
    }
    return Get-ComposePort
}

# The published port, read from the compose file rather than assumed.
function Get-ComposePort {
    $m = Select-String -Path 'docker-compose.yml' -Pattern '127\.0\.0\.1:(\d+):7070' | Select-Object -First 1
    if ($m) { return $m.Matches[0].Groups[1].Value }
    return '7070'
}

# Called last, so a truncated download runs nothing at all.
Main
