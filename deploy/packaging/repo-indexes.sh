#!/usr/bin/env bash
# The apt and dnf repository indexes (P11a step 3), served from GitHub
# Pages (D-26). Unsigned until step 5 gives them the release key.
#
#   repo-indexes.sh apt DIR   DIR/pool/*.deb  ->  DIR/dists/stable/...
#   repo-indexes.sh dnf DIR   DIR/*.rpm       ->  DIR/repodata/
#
# Every release stays in the pool: a downgrade is the package manager's
# (plan 3.9). apt needs apt-utils (`apt-ftparchive`), dnf needs
# createrepo_c.
set -euo pipefail
kind=${1:?usage: repo-indexes.sh apt|dnf DIR}
dir=${2:?usage: repo-indexes.sh apt|dnf DIR}
case "$kind" in
apt)
    cd "$dir"
    arch=binary-amd64
    mkdir -p "dists/stable/main/$arch"
    apt-ftparchive packages pool >"dists/stable/main/$arch/Packages"
    gzip -9 -kf "dists/stable/main/$arch/Packages"
    apt-ftparchive \
        -o APT::FTPArchive::Release::Origin=wanderbooks \
        -o APT::FTPArchive::Release::Suite=stable \
        -o APT::FTPArchive::Release::Codename=stable \
        -o APT::FTPArchive::Release::Architectures=amd64 \
        -o APT::FTPArchive::Release::Components=main \
        release dists/stable >dists/stable/Release.tmp
    mv dists/stable/Release.tmp dists/stable/Release
    ;;
dnf)
    createrepo_c --update "$dir"
    ;;
*)
    echo "usage: repo-indexes.sh apt|dnf DIR" >&2
    exit 2
    ;;
esac
