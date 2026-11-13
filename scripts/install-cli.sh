#!/bin/zsh
set -euo pipefail

# Install a `notan` command that opens a folder in the app bundle.
#
#   ./scripts/install-cli.sh                 # install to ~/.local/bin
#   NOTAN_BIN=/usr/local/bin ./scripts/install-cli.sh
#
# Then: notan .        (open the current folder)
#       notan ~/project

root="$(cd "$(dirname "$0")/.." && pwd)"
app="${NOTAN_APP:-$root/dist/Notan.app}"
bin="${NOTAN_BIN:-$HOME/.local/bin}"

if [[ ! -d "$app" ]]; then
  echo "app bundle missing: $app" >&2
  echo "run ./scripts/bundle.sh first" >&2
  exit 1
fi

mkdir -p "$bin"

cat > "$bin/notan" <<EOF
#!/bin/zsh
set -euo pipefail
app="$app"

if [[ \$# -eq 0 ]]; then
  open -n "\$app"
  exit 0
fi

dir="\$1"
if [[ ! -d "\$dir" ]]; then
  echo "not a folder: \$dir" >&2
  exit 1
fi
dir="\$(cd "\$dir" && pwd)"
open -n "\$app" --args "\$dir"
EOF

chmod +x "$bin/notan"
echo "installed $bin/notan"

case ":$PATH:" in
  *":$bin:"*) ;;
  *) echo "add $bin to PATH to use 'notan' from any directory" ;;
esac
