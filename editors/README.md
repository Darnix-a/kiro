# Editing Kiro files

Each of these setups associates `.kr` files with Kiro syntax highlighting.

## VS Code

From the Kiro checkout, open a window with the local extension loaded.

```sh
code --extensionDevelopmentPath="$PWD/editors/vscode" monitor.kr
```

The extension uses the [TextMate grammar](vscode/syntaxes/kiro.tmLanguage.json).
For regular VS Code windows, copy the directory into your local extensions
folder and reload VS Code.

```sh
mkdir -p ~/.vscode/extensions
cp -R editors/vscode ~/.vscode/extensions/darnix-a.kiro-language-0.1.0
```

## Kate

Install the [Kate syntax definition](kate/kiro.xml), then restart Kate.

```sh
install -Dm644 editors/kate/kiro.xml \
  "${XDG_DATA_HOME:-$HOME/.local/share}/org.kde.syntax-highlighting/syntax/kiro.xml"
```

## Neovim and Vim

The [Vim package](vim/) includes the filetype rule and syntax file. Link it
into your editor's package directory from the Kiro checkout.

```sh
nvim_packages="${XDG_DATA_HOME:-$HOME/.local/share}/nvim/site/pack/kiro/start"
mkdir -p "$nvim_packages"
ln -s "$PWD/editors/vim" "$nvim_packages/kiro"
```

For Vim, use `~/.vim/pack/kiro/start` as the package directory and enable
`filetype plugin indent on` and `syntax enable` in your Vim configuration.
