# Editing Kiro files

The editor support associates `.kr` files with Kiro and highlights the syntax.
You can install it without building the `kr` interpreter.

## VS Code

Download the `.vsix` file from the [latest Kiro release](https://github.com/Darnix-a/kiro/releases/latest).
Open **Extensions: Install from VSIX...** in the Command Palette and select it.
You can also run
`code --install-extension ./kiro-language-0.1.1.vsix`.

From the Kiro checkout, install into VS Code with Node.js 22 or newer and npm.

```sh
./editors/install.sh vscode
```

Use `code-insiders` or `codium` in place of `vscode` for those editors.
Reload the window and open a `.kr` file. The language indicator should say
**Kiro**. If it says **Plain Text**, check that **Kiro** is enabled in the
current profile and select **Kiro** from the language picker.

## Kate

```sh
./editors/install.sh kate
```

Restart Kate and open a `.kr` file. For Flatpak Kate, run the same command
with `XDG_DATA_HOME="$HOME/.var/app/org.kde.kate/data"`.

## Neovim and Vim

```sh
./editors/install.sh nvim
./editors/install.sh vim
```

Run the command for your editor, then restart it. The installer uses the
editor's native package directory. For Vim, if you use a custom vimrc,
make sure it enables `filetype plugin indent on` and `syntax enable`.
Open a `.kr` file and run `:set filetype?` to check that it says `kiro`.
