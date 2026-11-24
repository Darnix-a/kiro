#!/bin/sh
set -eu

usage() {
    printf '%s\n' 'Usage: editors/install.sh vscode|code-insiders|codium|kate|vim|nvim'
}

fail() {
    printf 'editors/install.sh: %s\n' "$*" >&2
    exit 1
}

[ "$#" -eq 1 ] || { usage >&2; exit 2; }
editor_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
data_home=${XDG_DATA_HOME:-${HOME:?Set HOME before running the installer}/.local/share}

case "$1" in
    -h|--help) usage; exit 0 ;;
    vscode|code|code-insiders|codium)
        cli=$1
        if [ "$cli" = vscode ]; then
            cli=code
        fi
        command -v "$cli" >/dev/null 2>&1 || fail "$cli was not found on PATH"
        command -v npx >/dev/null 2>&1 || fail 'install Node.js 22+ with npm to package the extension, or install a prebuilt VSIX'
        temp_dir=$(mktemp -d)
        vsix="$temp_dir/kiro-language.vsix"
        trap 'rm -f -- "$vsix"; rmdir -- "$temp_dir"' 0
        (
            cd -- "$editor_dir/vscode"
            npx --yes @vscode/vsce@4.1.1 package --no-dependencies --out "$vsix"
        )
        "$cli" --install-extension "$vsix" --force
        printf '%s\n' 'Reload the editor and open a .kr file.'
        ;;
    kate)
        syntax_dir="$data_home/org.kde.syntax-highlighting/syntax"
        mkdir -p -- "$syntax_dir"
        install -m 644 "$editor_dir/kate/kiro.xml" "$syntax_dir/kiro.xml"
        printf 'Installed %s/kiro.xml. Restart Kate.\n' "$syntax_dir"
        ;;
    vim|nvim)
        if [ "$1" = nvim ]; then
            package_root="$data_home/${NVIM_APPNAME:-nvim}/site/pack"
        else
            package_root="${HOME:?Set HOME before running the installer}/.vim/pack"
        fi
        package_dir="$package_root/kiro/start/kiro"
        # Earlier instructions linked this package to the checkout.
        if [ -L "$package_dir" ]; then
            linked_dir=$(CDPATH='' cd -- "$package_dir" 2>/dev/null && pwd -P) ||
                fail "existing package link is broken: $package_dir"
            [ "$linked_dir" = "$editor_dir/vim" ] ||
                fail "existing package links to $linked_dir; move it before installing here"
        else
            mkdir -p -- "$package_dir/ftdetect" "$package_dir/syntax"
            install -m 644 "$editor_dir/vim/ftdetect/kiro.vim" "$package_dir/ftdetect/kiro.vim"
            install -m 644 "$editor_dir/vim/syntax/kiro.vim" "$package_dir/syntax/kiro.vim"
        fi
        printf 'Kiro package available at %s. Restart %s.\n' "$package_dir" "$1"
        ;;
    *) usage >&2; exit 2 ;;
esac
