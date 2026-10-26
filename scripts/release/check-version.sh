#!/bin/sh
# Prints the version in Cargo.toml. With an argument (a tag without its "v"), fails unless they match.
set -eu

root="$(cd "$(dirname "$0")/../.." && pwd)"
version="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -n 1)"

if [ -z "$version" ]; then
  echo "error: no version found in Cargo.toml" >&2
  exit 1
fi

if [ "$#" -ge 1 ] && [ "$1" != "$version" ]; then
  echo "error: tag says $1 but Cargo.toml says $version" >&2
  exit 1
fi

echo "$version"
