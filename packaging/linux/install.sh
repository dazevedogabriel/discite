#!/bin/sh
# Instala o Discite só para o usuário atual (sem sudo): o executável em
# ~/.local/bin, o ícone e o atalho no menu de aplicativos.
# Para desinstalar: rm ~/.local/bin/discite ~/.local/share/applications/discite.desktop ~/.local/share/icons/hicolor/256x256/apps/discite.png
set -e
here=$(cd "$(dirname "$0")" && pwd)
bin="$HOME/.local/bin"
apps="$HOME/.local/share/applications"
icons="$HOME/.local/share/icons/hicolor/256x256/apps"
mkdir -p "$bin" "$apps" "$icons"
install -m 755 "$here/discite" "$bin/discite"
install -m 644 "$here/discite.png" "$icons/discite.png"
sed "s|^Exec=.*|Exec=$bin/discite|" "$here/discite.desktop" > "$apps/discite.desktop"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$apps" || true
echo "Discite installed: $bin/discite (also in the applications menu)."
