#!/usr/bin/env sh
# Installs a prebuilt Loadout binary from GitHub Releases.
#
#   curl -fsSL https://raw.githubusercontent.com/JacobRoedel/loadout/main/install.sh | sh
#
# Override the version or install location with environment variables:
#   LOADOUT_VERSION=v0.2.0 LOADOUT_INSTALL_DIR="$HOME/bin" sh install.sh
set -eu

repo="JacobRoedel/loadout"
version="${LOADOUT_VERSION:-latest}"
install_dir="${LOADOUT_INSTALL_DIR:-$HOME/.local/bin}"

os() { uname -s | tr '[:upper:]' '[:lower:]'; }
arch() { uname -m; }

target() {
  case "$(os)" in
    linux)
      case "$(arch)" in
        x86_64 | amd64) echo "x86_64-unknown-linux-gnu" ;;
        aarch64 | arm64) echo "aarch64-unknown-linux-gnu" ;;
        *)
          echo "loadout: unsupported architecture '$(arch)' on Linux" >&2
          exit 1
          ;;
      esac
      ;;
    darwin)
      case "$(arch)" in
        x86_64) echo "x86_64-apple-darwin" ;;
        arm64) echo "aarch64-apple-darwin" ;;
        *)
          echo "loadout: unsupported architecture '$(arch)' on macOS" >&2
          exit 1
          ;;
      esac
      ;;
    *)
      echo "loadout: unsupported OS '$(os)'; on Windows, download a release asset manually from" >&2
      echo "  https://github.com/$repo/releases" >&2
      exit 1
      ;;
  esac
}

main() {
  target_triple="$(target)"
  if [ "$version" = "latest" ]; then
    url="https://github.com/$repo/releases/latest/download/loadout-${target_triple}.tar.gz"
  else
    url="https://github.com/$repo/releases/download/${version}/loadout-${target_triple}.tar.gz"
  fi

  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' EXIT INT TERM

  echo "loadout: downloading $url"
  if ! curl -fsSL "$url" -o "$tmp_dir/loadout.tar.gz"; then
    echo "loadout: download failed; check that a release exists for '$version' and '$target_triple'" >&2
    echo "  https://github.com/$repo/releases" >&2
    exit 1
  fi

  tar xzf "$tmp_dir/loadout.tar.gz" -C "$tmp_dir"
  binary="$(find "$tmp_dir" -type f -name loadout | head -n 1)"
  if [ -z "$binary" ]; then
    echo "loadout: could not find a 'loadout' binary in the downloaded archive" >&2
    exit 1
  fi

  mkdir -p "$install_dir"
  install -m 0755 "$binary" "$install_dir/loadout"

  echo "loadout: installed to $install_dir/loadout"
  case ":$PATH:" in
    *":$install_dir:"*) ;;
    *)
      echo ""
      echo "$install_dir is not on your PATH. Add it, for example:"
      echo "  export PATH=\"$install_dir:\$PATH\""
      ;;
  esac
}

main
