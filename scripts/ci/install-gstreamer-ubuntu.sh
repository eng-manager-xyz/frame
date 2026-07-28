#!/usr/bin/env bash
set -euo pipefail

packages=(
  libgstreamer1.0-dev
  libgstreamer-plugins-base1.0-dev
  gstreamer1.0-plugins-base
  gstreamer1.0-plugins-good
)

while (($# > 0)); do
  case "$1" in
    --libav)
      packages+=(gstreamer1.0-libav)
      ;;
    --tools)
      packages+=(gstreamer1.0-tools)
      ;;
    --base-apps)
      packages+=(gstreamer1.0-plugins-base-apps)
      ;;
    --ripgrep)
      packages+=(ripgrep)
      ;;
    *)
      echo "unknown option: $1" >&2
      exit 64
      ;;
  esac
  shift
done

# GitHub's Ubuntu images can retain the Azure archive endpoint even when that
# mirror is delivering package bodies too slowly for the bounded media jobs.
# Use the canonical Ubuntu archive while retaining apt's normal signed metadata.
for source_file in \
  /etc/apt/sources.list \
  /etc/apt/sources.list.d/ubuntu.sources
do
  if [[ -f "${source_file}" ]] \
    && grep -Eq 'https?://azure\.archive\.ubuntu\.com/ubuntu/?' "${source_file}"
  then
    sudo sed -Ei \
      's#https?://azure\.archive\.ubuntu\.com/ubuntu/?#http://archive.ubuntu.com/ubuntu/#g' \
      "${source_file}"
  fi
done

apt_options=(
  -o Acquire::Retries=5
  -o Acquire::http::Timeout=30
  -o Acquire::https::Timeout=30
  -o Acquire::ForceIPv4=true
)

sudo apt-get "${apt_options[@]}" update
sudo env DEBIAN_FRONTEND=noninteractive \
  apt-get "${apt_options[@]}" install -y --no-install-recommends "${packages[@]}"
