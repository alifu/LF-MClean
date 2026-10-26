#!/bin/sh
# Prints the Homebrew formula for a release: make-formula.sh <version> <sha256 of the tarball>
set -eu

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <version> <sha256>" >&2
  exit 2
fi
version="$1"
sha="$2"

case "$version" in
  *[!0-9A-Za-z.+-]* | "") echo "error: bad version: $version" >&2; exit 2 ;;
esac
case "$sha" in
  *[!0-9a-f]* | "") echo "error: sha256 must be lowercase hex" >&2; exit 2 ;;
esac
if [ "${#sha}" -ne 64 ]; then
  echo "error: sha256 must be 64 characters" >&2
  exit 2
fi

cat <<RUBY
class Lfmclean < Formula
  desc "Interactive cleaner for Xcode, Android, Gradle and Kotlin caches"
  homepage "https://github.com/alifu/LF-MClean"
  url "https://github.com/alifu/LF-MClean/releases/download/v${version}/lfmclean-${version}-macos-universal.tar.gz"
  sha256 "${sha}"
  license "MIT"

  depends_on :macos

  def install
    bin.install "lfmclean"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/lfmclean --version")
  end
end
RUBY
