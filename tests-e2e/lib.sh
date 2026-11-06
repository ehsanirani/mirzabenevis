# Source me. Gives `mz` = mirzabenevis-cli in an isolated, empty environment.
#   T          scratch root (fresh per sourcing)
#   MZ_PATH    PATH for the child (default: a dir with only coreutils-ish tools)
#   MZ_ENV     extra "K=V" words passed to env -i
BIN=${BIN:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/target/debug/mirzabenevis-cli}
T=$(mktemp -d "${TMPDIR:-/tmp}/mzb-e2e.XXXXXX")
mkdir -p "$T/home" "$T/xdg" "$T/bin"
MZ_PATH=${MZ_PATH:-"$T/bin:/usr/bin:/bin"}
cfgfile() { echo "$T/xdg/mirzabenevis/config.toml"; }
writecfg() { mkdir -p "$T/xdg/mirzabenevis"; cat > "$(cfgfile)"; }
mz() { env -i HOME="$T/home" XDG_CONFIG_HOME="$T/xdg" PATH="$MZ_PATH" $MZ_ENV "$BIN" "$@"; }
# Guard: the login shell's /etc/profile puts the real, logged-in `claude` on PATH.
# Shadow it so tests never make real calls by accident.
printf '#!/bin/sh\necho "guard claude: real claude blocked in tests" >&2\nexit 99\n' > "$T/bin/claude"; chmod +x "$T/bin/claude"
