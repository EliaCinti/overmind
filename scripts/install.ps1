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

    # 3 — the directory IS the installation (ADR-0047): the compose file lives
    #     here and so do .\data and .\agent.
    $dir = if ($env:OVERMIND_HOME) { $env:OVERMIND_HOME } else { Join-Path $HOME 'overmind' }
    if (Test-Path (Join-Path $dir 'data\overmind.sqlite')) {
        Write-Host "There is already an Overmind in $dir - updating it in place."
    }
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    Set-Location $dir

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
    $port = Get-ComposePort
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
        $code = (Get-Content $codeFile -Raw).Trim()
        Write-Host ""
        Write-Host "  The first claim costs this code, and it is asked for once:"
        Write-Host ""
        Write-Host "      $code"
        Write-Host ""
    } elseif (Test-Path (Join-Path $dir 'data\overmind.sqlite')) {
        Write-Host ""
        Write-Host "  Already claimed - no setup code to give."
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

# The published port, read from the compose file rather than assumed.
function Get-ComposePort {
    $m = Select-String -Path 'docker-compose.yml' -Pattern '127\.0\.0\.1:(\d+):7070' | Select-Object -First 1
    if ($m) { return $m.Matches[0].Groups[1].Value }
    return '7070'
}

# Called last, so a truncated download runs nothing at all.
Main
