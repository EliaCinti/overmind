#!/bin/sh
# Overmind's installer (ADR-0045). POSIX sh: macOS and Linux.
#
#   curl -fsSL https://overmind.eliacinti.dev/install.sh | sh
#
# Or, if you would rather read it first — and you should, it is somebody
# else's shell script:
#
#   curl -fsSLO https://github.com/EliaCinti/overmind/releases/latest/download/install.sh
#   curl -fsSLO https://github.com/EliaCinti/overmind/releases/latest/download/install.sh.sha256
#   shasum -a 256 -c install.sh.sha256 && less install.sh && sh install.sh
#
# It does seven things and nothing else. It never installs a container engine
# behind your back, and it writes only inside the directory it makes.
set -eu

# Stamped by the release workflow. A script from release X fetches release X's
# compose file and checks it against the digest that release published, so the
# file and its expected hash come from one place.
TAG="__OVERMIND_TAG__"
COMPOSE_SHA256="__OVERMIND_COMPOSE_SHA256__"

main() {
    # Stamped, or nothing. The guard tests the SHAPE of the values, never the
    # sentinel: the release step rewrites EVERY occurrence, so a guard that
    # compares the value against that sentinel becomes a value compared with
    # itself -- false, and the whole digest check skipped. That shipped in the
    # first draft of this script and would have published an installer that
    # verified nothing.
    case "$TAG" in
        v[0-9]*) : ;;
        *) die "This installer was never stamped with a release. Take it from a release, not from the repository:
    https://github.com/EliaCinti/overmind/releases/latest" ;;
    esac
    case "$COMPOSE_SHA256" in
        ????????????????????????????????????????????????????????????????) : ;;
        *) die "This installer carries no digest for the compose file, so it cannot check what it downloads. Take it from a release." ;;
    esac

    say "Overmind ${TAG}"

    # 1 — what we are on.
    os=$(uname -s)
    arch=$(uname -m)
    case "$os" in
        Linux|Darwin) : ;;
        *)
            die "This installer runs on Linux and macOS. On Windows use install.ps1, or run this inside WSL2."
            ;;
    esac
    case "$arch" in
        x86_64|amd64|arm64|aarch64) : ;;
        *)
            die "The published image is built for x86_64 and arm64; this machine is ${arch}. You can still build from source — see the README."
            ;;
    esac

    # 2 — ask the engine, not the filesystem. `command -v docker` passes in the
    #     two cases that actually happen, and both fail later and worse.
    if ! command -v docker >/dev/null 2>&1; then
        case "$os" in
            Linux)
                die "Docker is not installed. Get it once with:
    curl -fsSL https://get.docker.com | sh
    sudo usermod -aG docker \$USER && newgrp docker
then run this installer again."
                ;;
            Darwin)
                die "Docker is not installed. Either install Docker Desktop and open it once, or stay in the terminal with:
    brew install colima docker docker-compose && colima start --cpu 4 --memory 8
then run this installer again."
                ;;
        esac
    fi
    if ! docker info >/dev/null 2>&1; then
        if docker info 2>&1 | grep -qi "permission denied"; then
            die "Docker is installed but this user cannot reach it. You were added to the 'docker' group but have not logged in since:
    newgrp docker
or open a new terminal, then run this installer again."
        fi
        case "$os" in
            Darwin) die "Docker is installed but not running. Open Docker Desktop (or 'colima start'), wait for it to say it is ready, then run this installer again." ;;
            Linux)  die "Docker is installed but not running. Start it with 'sudo systemctl start docker', then run this installer again." ;;
        esac
    fi
    if ! docker compose version >/dev/null 2>&1; then
        die "This Docker has no 'compose' subcommand. Install the Compose plugin (on Debian/Ubuntu: 'sudo apt install docker-compose-plugin'), then run this installer again."
    fi

    # 3 — the directory. It IS the installation (ADR-0047): the compose file
    #     lives here and so do ./data and ./agent, so moving this folder moves
    #     the whole Overmind.
    #
    #     It lands beside you, in ./overmind, because that is where somebody who
    #     just ran a command goes looking for what it made. A subfolder rather
    #     than the bare working directory: a compose file, a data/ and an agent/
    #     scattered into whatever you happened to be in is not a gift.
    if [ -n "${OVERMIND_HOME:-}" ]; then
        dir=$OVERMIND_HOME
        chosen=yes
    else
        dir=$PWD/overmind
        chosen=no
    fi
    previous=$HOME/overmind
    if [ -e "$dir/data/overmind.sqlite" ]; then
        say "There is already an Overmind in $dir — updating it in place."
    elif [ "$chosen" = no ] && [ "$dir" != "$previous" ] && [ -e "$previous/data/overmind.sqlite" ]; then
        # The hazard ADR-0047 names, met head-on: a folder IS an instance, so
        # installing into a new one beside an existing instance does not update
        # it, it silently replaces it with an empty one and leaves the real
        # database where nobody is looking.
        die "You already have an Overmind in $previous, and installing here would
    make a second, empty one instead of updating that one.

    To update the one you have:
        cd $previous && docker compose pull && docker compose up -d

    To move it here, with it stopped:
        cd $previous && docker compose down
        mv $previous $dir
        cd $dir && docker compose up -d

    To install a second one here on purpose:
        OVERMIND_HOME=$dir  … before the install command"
    fi
    mkdir -p "$dir"
    cd "$dir"

    # 4 — the compose file, from this script's own release, checked.
    url="https://github.com/EliaCinti/overmind/releases/download/${TAG}/docker-compose.yml"
    say "Fetching the compose file for ${TAG}…"
    if [ -f docker-compose.yml ]; then
        cp docker-compose.yml docker-compose.yml.previous
    fi
    curl -fsSL "$url" -o docker-compose.yml.new || die "Could not download $url"
    got=$(sha256 docker-compose.yml.new)
    if [ "$got" != "$COMPOSE_SHA256" ]; then
        rm -f docker-compose.yml.new
        die "The compose file does not match the digest this installer carries.
    expected  $COMPOSE_SHA256
    got       $got
Nothing was installed. Download the release by hand if you want to look."
    fi
    mv docker-compose.yml.new docker-compose.yml

    # 5 — start it. The same line is the update.
    say "Starting Overmind…"
    docker compose up -d --pull always

    # 6 — wait for it to answer, then the URL and the code. From the file the
    #     server wrote, never from the log: on the second run the log still
    #     holds the first run's line.
    port=$(compose_port)
    i=0
    while [ "$i" -lt 90 ]; do
        if curl -fsS -m 2 -o /dev/null "http://127.0.0.1:${port}/" 2>/dev/null; then
            break
        fi
        i=$((i + 1))
        sleep 1
    done

    printf '\n'
    say "Overmind is running at http://localhost:${port}"
    # Three states, and they must not be confused. The file being unreadable
    # is the ORDINARY Linux case -- it is 0600 and root's, and `sudo -n` will
    # not prompt -- so an earlier draft fell through to "already claimed" and
    # told a brand-new user something false, in the one step this installer
    # exists to make easy.
    if [ -e "$dir/data/setup-code" ]; then
        code=$(read_setup_code)
        if [ -n "$code" ]; then
            printf '\n  The first claim costs this code, and it is asked for once:\n\n      %s\n\n' "$code"
        else
            printf '\n  The first claim costs a code. It is root'"'"'s, so read it with:\n\n      sudo cat %s/data/setup-code\n\n' "$dir"
        fi
    elif [ -e "$dir/data/overmind.sqlite" ]; then
        printf '\n  Already claimed — no setup code to give.\n\n'
    else
        printf '\n  No setup code was written. The server log says why:\n\n      docker compose logs overmind\n\n'
    fi

    # 7 — the three things that come next.
    cat <<EOF
  Next:
    • Open http://localhost:${port}, create the owner account, paste that code.
    • Give the agent a way to pay — sign in with your Claude subscription from
      the notice above the first screen, or export ANTHROPIC_API_KEY before
      starting. (This installer cannot ask: piped to sh, its input is the pipe.)
    • Your instance lives in $dir — the compose file, ./data and ./agent.
      Moving that folder moves everything. 'docker compose down -v' cannot
      touch it. The way out is an archive: the Archive button, owner only.

  To update, from $dir:
      docker compose up -d --pull always
EOF
}

say() { printf '%s\n' "$*"; }
die() { printf '\n%s\n\n' "$*" >&2; exit 1; }

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

# The published port, read from the compose file rather than assumed: somebody
# who edited it should still be told the truth.
compose_port() {
    p=$(sed -n 's/.*"\{0,1\}127\.0\.0\.1:\([0-9]\{1,\}\):7070"\{0,1\}.*/\1/p' docker-compose.yml | head -1)
    [ -n "$p" ] || p=7070
    printf '%s' "$p"
}

# 0600 and root's on Linux; try plainly first, and only then with sudo, and
# only if sudo will not prompt — an installer must not sit waiting for a
# password nobody can see.
read_setup_code() {
    f="$dir/data/setup-code"
    [ -e "$f" ] || return 0
    if [ -r "$f" ]; then
        tr -d '\r\n' < "$f"
    elif command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null; then
        sudo cat "$f" 2>/dev/null | tr -d '\r\n'
    fi
}

# Called last, so a truncated download runs nothing at all.
main "$@"
