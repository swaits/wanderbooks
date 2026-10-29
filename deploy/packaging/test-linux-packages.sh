#!/usr/bin/env bash
# linux_packages_install_and_upgrade (P11a step 3): in a clean Debian,
# Fedora and Arch container, install release 0.1.0 from its package,
# open the books as an ordinary user, upgrade to 0.1.1 as root, and check
# that the books' head is unchanged, that the host the books were open
# under restarted onto the new file, and that `wanderbooks update` refuses
# (a package manager owns the program). The containers run with
# seccomp=unconfined: the host refuses to run under a filter (host.rs
# INVARIANT 2), and Docker's default profile is one. Run from the repository root after
# `just test-linux-packages` built both releases (target/packages/).
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
old=0.1.0 new=0.1.1
pk=$PWD/target/packages
mount=(-v "$pk:/packages:ro")

# The steps every distro shares, run inside the container: `install` and
# `upgrade` are shell snippets for that distro.
common() { cat <<'INNER'
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
useradd -m tester
as_tester() { su tester -c "$*"; }
install_old
test -x /usr/bin/wanderbooks
grep -q . /usr/share/wanderbooks/install-method
as_tester 'wanderbooks --help' >/dev/null 2>&1 || true
as_tester 'wanderbooks setup' >/dev/null
as_tester 'wanderbooks books create acme' >/dev/null
head_of() { as_tester 'tail -n1 ~/.local/share/wanderbooks/books/acme/journal.jsonl | sha256sum'; }
before=$(as_tester 'wanderbooks books list')
head_before=$(head_of)
echo "before: $before  head $head_before"
# The books open: a holder keeps the host and server running.
as_tester 'nohup wanderbooks serve >/tmp/serve.log 2>&1 &'
for _ in $(seq 50); do pgrep -u tester -x wanderbooks -a | grep -q ' host' && break; sleep 0.2; done
host_before=$(pgrep -u tester -f 'wanderbooks host' | head -n1)
test -n "$host_before"
upgrade_new
# The host notices the replaced file and restarts onto it.
for _ in $(seq 150); do
    host_after=$(pgrep -u tester -f 'wanderbooks host' | head -n1 || true)
    [ -n "$host_after" ] && [ "$host_after" != "$host_before" ] && break
    sleep 0.2
done
test -n "$host_after" && test "$host_after" != "$host_before"
! readlink "/proc/$host_after/exe" | grep -q deleted
after=$(as_tester 'wanderbooks books list')
echo "after:  $after  head $(head_of)"
test "$before" = "$after"
test "$head_before" = "$(head_of)"
if as_tester 'wanderbooks update' >/tmp/update.out 2>&1; then echo "update did not refuse" >&2; exit 1; fi
grep -q 'package manager' /tmp/update.out
as_tester 'wanderbooks close' >/dev/null 2>&1 || true
echo "OK"
INNER
}

run() { # name image install_old upgrade_new
    local name=$1 image=$2
    echo "== $name ($image)"
    docker run --rm --security-opt seccomp=unconfined -e DEBIAN_FRONTEND=noninteractive "${mount[@]}" "$image" bash -c "
$3
$4
$(common)"
}

# Debian and Fedora install from a repository built by repo-indexes.sh
# (the one GitHub Pages serves), the new release added to it afterwards.
mount+=(-v "$PWD/deploy/packaging:/tools:ro")
run debian debian:trixie-slim \
    "apt-get update -qq && apt-get install -y -qq apt-utils procps >/dev/null
mkdir -p /repo/apt/pool && cp /packages/$old/*.deb /repo/apt/pool/ && /tools/repo-indexes.sh apt /repo/apt
echo 'deb [trusted=yes] file:///repo/apt stable main' >/etc/apt/sources.list.d/wanderbooks.list
install_old() { apt-get update -qq && apt-get install -y -qq wanderbooks >/dev/null; }" \
    "upgrade_new() { cp /packages/$new/*.deb /repo/apt/pool/ && /tools/repo-indexes.sh apt /repo/apt && apt-get update -qq && apt-get install -y -qq --only-upgrade wanderbooks >/dev/null; }"

run fedora fedora:latest \
    "dnf install -y -q procps-ng util-linux createrepo_c >/dev/null
mkdir -p /repo/dnf && cp /packages/$old/*.rpm /repo/dnf/ && /tools/repo-indexes.sh dnf /repo/dnf
printf '[wanderbooks]\nname=wanderbooks\nbaseurl=file:///repo/dnf\ngpgcheck=0\n' >/etc/yum.repos.d/wanderbooks.repo
install_old() { dnf install -y -q wanderbooks >/dev/null; }" \
    "upgrade_new() { cp /packages/$new/*.rpm /repo/dnf/ && /tools/repo-indexes.sh dnf /repo/dnf && dnf upgrade -y -q --refresh wanderbooks >/dev/null; }"

# Arch: the AUR's own PKGBUILD (deploy/packaging/aur), its source pointed at
# the tarball just built instead of the release URL, makes the package as
# an ordinary user.
arch_build='build() { rm -rf /build/$1 && mkdir -p /build/$1 && cp /packages/$1/*.tar.gz /build/$1/ && cp /tools/aur/PKGBUILD /build/$1/PKGBUILD && sed -i "s/^pkgver=.*/pkgver=$1/; s|^source=.*|source=(wanderbooks_$1_x86_64.tar.gz)|" /build/$1/PKGBUILD && chown -R builder /build/$1 && su builder -c "cd /build/$1 && makepkg -f --nodeps >/dev/null 2>&1"; }'
run arch archlinux:latest \
    "useradd -m builder; pacman -Sy --noconfirm --needed base-devel procps-ng >/dev/null 2>&1; $arch_build; install_old() { build $old; pacman -U --noconfirm /build/$old/*.pkg.tar.zst >/dev/null 2>&1; }" \
    "upgrade_new() { build $new; pacman -U --noconfirm /build/$new/*.pkg.tar.zst >/dev/null 2>&1; }"
echo "all packages install and upgrade"
