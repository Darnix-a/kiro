#!/bin/sh
set -eu

fail() {
    printf 'install.sh: %s\n' "$*" >&2
    exit 1
}

prefix="${HOME:?Set HOME before running the installer}/.local"
configure_path=true

while [ "$#" -gt 0 ]; do
    case "$1" in
        --prefix)
            [ "$#" -ge 2 ] && [ -n "$2" ] || fail "--prefix needs a directory"
            prefix=$2
            shift
            ;;
        --no-path) configure_path=false ;;
        -h|--help)
            printf '%s\n' 'Usage: ./install.sh [--prefix DIRECTORY] [--no-path]'
            exit 0
            ;;
        *) fail "unknown option: $1" ;;
    esac
    shift
done

source_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
[ -f "$source_dir/Cargo.toml" ] || fail "run the installer from a Kiro checkout"

mkdir -p -- "$prefix"
prefix=$(CDPATH='' cd -- "$prefix" && pwd -P)
bin_dir="$prefix/bin"
newline='
'
case "$bin_dir" in
    *:*|*"$newline"*) fail "installation path cannot contain a colon or newline" ;;
esac

cargo_bin_dir="${CARGO_HOME:-$HOME/.cargo}/bin"
if ! command -v cargo >/dev/null 2>&1; then
    if [ ! -x "$cargo_bin_dir/cargo" ]; then
        command -v curl >/dev/null 2>&1 || fail "install Cargo or curl first"
        rustup_init=$(mktemp)
        trap 'rm -f -- "$rustup_init"' 0
        printf '%s\n' 'Installing Rust with rustup...'
        curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs -o "$rustup_init"
        sh "$rustup_init" -y --profile minimal --default-toolchain stable --no-modify-path
    fi
    export PATH="$cargo_bin_dir${PATH:+:$PATH}"
fi

cargo install --locked --path "$source_dir" --bin kr --root "$prefix"
"$bin_dir/kr" --version

add_to_config() {
    config=$1
    line=$2
    if [ -f "$config" ] && grep -Fqx -- "$line" "$config"; then
        return
    fi
    mkdir -p -- "$(dirname -- "$config")"
    printf '\n# Kiro\n%s\n' "$line" >> "$config"
    printf 'Updated %s\n' "$config"
}

if [ "$configure_path" = true ]; then
    # Quote the path as a single word, including paths containing apostrophes.
    quoted_bin="'$(printf '%s' "$bin_dir" | sed "s/'/'\\\\''/g")'"
    path_line="case \":\${PATH-}:\" in *:$quoted_bin:*) ;; *) export PATH=$quoted_bin\${PATH:+\":\$PATH\"} ;; esac"
    login_shell=${SHELL:-sh}
    case "${login_shell##*/}" in
        bash)
            add_to_config "$HOME/.bashrc" "$path_line"
            if [ -f "$HOME/.bash_profile" ]; then
                add_to_config "$HOME/.bash_profile" "$path_line"
            elif [ -f "$HOME/.bash_login" ]; then
                add_to_config "$HOME/.bash_login" "$path_line"
            else
                add_to_config "$HOME/.profile" "$path_line"
            fi
            ;;
        zsh)
            add_to_config "${ZDOTDIR:-$HOME}/.zshrc" "$path_line"
            ;;
        fish)
            fish_path=$("$login_shell" --no-config -c 'string escape -- $argv[1]' "$bin_dir")
            add_to_config "${XDG_CONFIG_HOME:-$HOME/.config}/fish/config.fish" \
                "fish_add_path --path -- $fish_path"
            ;;
        *)
            add_to_config "$HOME/.profile" "$path_line"
            ;;
    esac
    printf '%s\n' 'Open a new terminal and run kr --version.'
else
    printf 'Installed %s/kr\n' "$bin_dir"
fi
