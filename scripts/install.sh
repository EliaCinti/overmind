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

    # 3 — the directory, and the identity that makes it an instance of its own.
    #
    #     It lands beside whoever ran the command, in ./overmind, because that
    #     is where somebody goes looking for what a command just made. A
    #     subfolder rather than the bare working directory: scattering a
    #     compose file, a data/ and an agent/ into whatever directory you
    #     happened to be in is not a gift.
    dir=${OVERMIND_HOME:-"$PWD/overmind"}
    mkdir -p "$dir" 2>/dev/null || die "Cannot create $dir — this directory is not writable by you.
    Run it somewhere you own, or name the place yourself:

        curl -fsSL https://overmind.eliacinti.dev/install.sh | OVERMIND_HOME=\$HOME/overmind sh

    (the variable goes before 'sh', not before 'curl': in a pipeline each side
    is its own process, and curl is not the one that needs it.)"
    cd "$dir" || die "Cannot enter $dir."
    #     Absolute from here on: the identity is derived from it, and every
    #     line printed below names it back to the person.
    dir=$PWD
    if [ -e "$dir/data/overmind.sqlite" ]; then
        say "There is already an Overmind in $dir — updating it in place."
    else
        elsewhere
    fi
    identify

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
    port=$(instance_port)
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
    • Your instance lives in $dir — the compose file, ./data, ./agent, and a
      .env holding this instance's name and port so a second install
      elsewhere is a second instance rather than a fight over this one.
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

# Every Overmind this engine knows of, running or stopped, wherever it lives
# and whatever its folder is called.
#
# Asked of Docker rather than guessed at by scanning paths: the /data bind
# mount names the install directory exactly, so an instance whose folder was
# renamed or moved is still found — and a folder merely CALLED `overmind`,
# with nothing in it, is correctly not one.
known_instances() {
    docker ps -a --format '{{.ID}} {{.Image}}' 2>/dev/null \
        | grep -i overmind | cut -d' ' -f1 \
        | while read -r c; do
            src=$(docker inspect --format '{{range .Mounts}}{{if eq .Destination "/data"}}{{.Source}}{{end}}{{end}}' "$c" 2>/dev/null)
            [ -n "$src" ] && printf '%s\n' "${src%/data}"
        done | sort -u
}

# Installing beside an Overmind that already exists is safe now — separate
# project, separate container, separate port — but it is rarely what somebody
# meant, and finding out later is expensive. So it is said before it happens,
# with the choice offered rather than assumed.
#
# Asked on /dev/tty, not stdin: piped to sh, stdin IS the pipe. Where there is
# no terminal — CI, a provisioning script — nothing is asked and the safe
# thing happens, which is the new instance nobody has to undo.
elsewhere() {
    others=$(known_instances | grep -v "^${dir}$" || true)
    [ -n "$others" ] || return 0

    printf '\n  Overmind is already installed on this machine:\n\n'
    printf '%s\n' "$others" | while read -r o; do printf '      %s\n' "$o"; done
    printf '\n  Installing here makes a SECOND, separate instance: its own data, its own\n'
    printf '  container, its own port. Nothing above is touched.\n\n'

    count=$(printf '%s\n' "$others" | wc -l | tr -d ' ')
    # A terminal on stdout is the honest test. `-r /dev/tty` is true even where
    # the device cannot be opened -- measured: it asked the question, printed
    # "Device not configured", and answered itself.
    if [ "$count" -ne 1 ] || [ ! -t 1 ]; then
        say "  Installing here. To update one of those instead, run this from its folder."
        return 0
    fi

    printf '    [u] update that one instead   [n] new instance here   [q] quit\n\n  > '
    read -r answer < /dev/tty 2>/dev/null || answer=n
    printf '\n'
    case "$answer" in
        u|U)
            dir=$(printf '%s\n' "$others" | head -1)
            cd "$dir" || die "Cannot enter $dir."
            say "Updating the Overmind in $dir."
            ;;
        q|Q) die "Nothing was installed." ;;
        *) say "Installing a second instance in $dir." ;;
    esac
}

# What makes THIS folder an instance rather than a name collision.
#
# Compose names a project after its directory, and since 3 Sep 2026 every
# default install IS a directory called `overmind` — so two of them would be
# one project: one set of containers, one published port, and an `up` in the
# second recreating the first onto this folder's empty ./data. The identity
# therefore comes from the whole path, written where Compose reads it.
#
# Written once and never rewritten: re-running here must not rename a project
# out from under containers that are already running under the old name.
identify() {
    [ -f .env ] && return 0
    if [ -e data/overmind.sqlite ]; then
        # Older than this file. Compose has been naming its containers after
        # the directory all along, so keep exactly that name — inventing a new
        # one here would orphan a container that is running right now.
        project=$(printf '%s' "${dir##*/}" | tr '[:upper:]' '[:lower:]' | tr -cd 'a-z0-9_-')
        [ -n "$project" ] || project=overmind
        port=$(compose_port)
    else
        project=overmind-$(path_slug)
        port=$(free_port)
    fi
    cat > .env <<EOF
# Written by the installer, and read by every 'docker compose' you run here.
# It is what makes this folder its own instance: Compose names a project after
# its directory, and every default install is a directory called 'overmind'.
# Two of them without this file would be ONE project — one set of containers,
# one port — and starting the second would recreate the first onto this
# folder's empty ./data.
#
# Moving this folder is still how you move the instance. Delete this file only
# if you also mean to give the instance a different name.
COMPOSE_PROJECT_NAME=$project
OVERMIND_NAME=$project
OVERMIND_PORT=$port
EOF
}

# Eight hex of the folder's full path: stable across runs, different for every
# folder, and short enough to read in `docker ps`.
path_slug() {
    if command -v sha256sum >/dev/null 2>&1; then
        printf '%s' "$dir" | sha256sum | cut -c1-8
    else
        printf '%s' "$dir" | shasum -a 256 | cut -c1-8
    fi
}

# Is somebody already on this port? Two questions, because neither alone is
# enough: something may answer HTTP without being a container, and a container
# may hold the port while still starting and answering nothing.
port_taken() {
    curl -fsS -m 1 -o /dev/null "http://127.0.0.1:$1/" 2>/dev/null && return 0
    docker ps --format '{{.Ports}}' 2>/dev/null | grep -q ":$1->" && return 0
    return 1
}

# 7070 when it is free — that is the address every document names. Otherwise a
# port belonging to THIS folder, derived from its path rather than from
# whatever happens to be listening at this second: "the first free one" hands
# the same number to two installs made while both were stopped, and they
# collide the day somebody starts both. Measured, on the first test of this.
free_port() {
    if ! port_taken 7070; then
        printf '7070'
        return 0
    fi
    n=$(printf '%s' "$dir" | cksum | cut -d' ' -f1)
    p=$((7071 + n % 29))
    i=0
    while [ "$i" -lt 29 ]; do
        if ! port_taken "$p"; then
            printf '%s' "$p"
            return 0
        fi
        p=$((7071 + (p - 7070) % 29))
        i=$((i + 1))
    done
    printf '7070'
}

# This instance's port: what the identity says, falling back to the compose
# file for an install that predates .env or a file somebody edited by hand.
instance_port() {
    p=$(sed -n 's/^OVERMIND_PORT=\([0-9]\{1,\}\).*/\1/p' .env 2>/dev/null | head -1)
    [ -n "$p" ] || p=$(compose_port)
    printf '%s' "$p"
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
