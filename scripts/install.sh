#!/bin/sh
# GitHub release distribution. No root, no remote script evaluation, no plaintext keys.
set -eu
umask 077
psychotalk_repo=fpoletto/Psychotalk
psychotalk_version=${PSYCHOTALK_VERSION:-v0.4.2-experimental}
case "$(uname -s)" in
 Darwin) psychotalk_system=macos; psychotalk_data="${PSYCHOTALK_DATA_DIR:-$HOME/Library/Application Support/Psychotalk}" ;;
 Linux) psychotalk_system=linux; psychotalk_data="${PSYCHOTALK_DATA_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/psychotalk}" ;;
 *) echo 'Sistema não suportado: use macOS ou Linux.' >&2; exit 1 ;;
esac
case "$(uname -m)" in arm64|aarch64) psychotalk_arch=arm64 ;; x86_64) psychotalk_arch=x86_64 ;; *) echo 'Arquitetura não suportada.' >&2; exit 1 ;; esac
case "$psychotalk_system-$psychotalk_arch" in macos-arm64|macos-x86_64|linux-x86_64) ;; *) echo 'Pacote não disponível para esta arquitetura.' >&2; exit 1 ;; esac
psychotalk_program="$psychotalk_data/program"
psychotalk_launcher="${PSYCHOTALK_BIN_DIR:-$HOME/.local/bin}/psychotalk"
case "$psychotalk_data" in /*) ;; *) echo 'O diretório de dados deve ser absoluto.' >&2; exit 1 ;; esac
if [ "${1:-}" = --uninstall ]; then
 if [ -x "$psychotalk_program/current/psychotalk" ]; then "$psychotalk_program/current/psychotalk" stop; fi
 if [ -f "$psychotalk_program/.owner" ] && [ "$(cat "$psychotalk_program/.owner")" = 'Psychotalk program v1' ]; then rm -rf "$psychotalk_program"; fi
 if [ -L "$psychotalk_launcher" ] && [ "$(readlink "$psychotalk_launcher")" = "$psychotalk_program/current/psychotalk" ]; then rm "$psychotalk_launcher"; fi
 echo "Programa removido. Cofre e chaveiro preservados em: $psychotalk_data"
 exit 0
fi
command -v gh >/dev/null 2>&1 || { echo 'Instale o GitHub CLI (gh) e execute gh auth login antes de instalar.' >&2; exit 1; }
gh auth status >/dev/null 2>&1 || { echo 'Execute gh auth login antes de instalar ou atualizar.' >&2; exit 1; }
if [ "${1:-}" = --update ]; then
 psychotalk_version=$(gh api "repos/$psychotalk_repo/releases" --jq '[.[] | select(.draft == false and any(.assets[]; .name == "install.sh"))][0].tag_name')
 case "$psychotalk_version" in v[0-9]*-experimental|v[0-9]*.[0-9]*) ;; *) echo 'Nenhuma versão desktop publicada.' >&2; exit 1 ;; esac
elif [ -n "${1:-}" ]; then echo 'Use install.sh, install.sh --update ou install.sh --uninstall.' >&2; exit 1
fi
printf '%s\n' "$psychotalk_version" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.]+)?$' || { echo 'Versão inválida.' >&2; exit 1; }
psychotalk_tmp=$(mktemp -d "${TMPDIR:-/tmp}/psychotalk-install.XXXXXX")
trap 'rm -rf "$psychotalk_tmp"' EXIT HUP INT TERM
psychotalk_package="psychotalk-${psychotalk_version#v}-$psychotalk_system-$psychotalk_arch.tar.gz"
gh release download "$psychotalk_version" --repo "$psychotalk_repo" --pattern "$psychotalk_package" --pattern SHA256SUMS --dir "$psychotalk_tmp"
cd "$psychotalk_tmp"
psychotalk_expected=$(awk -v name="$psychotalk_package" '$2 == name {print $1}' SHA256SUMS)
case "$psychotalk_expected" in ''|*[!a-f0-9]*) echo 'Checksum ausente ou inválido.' >&2; exit 1 ;; esac
[ "${#psychotalk_expected}" -eq 64 ] || exit 1
if command -v sha256sum >/dev/null 2>&1; then psychotalk_actual=$(sha256sum "$psychotalk_package" | awk '{print $1}'); else psychotalk_actual=$(shasum -a 256 "$psychotalk_package" | awk '{print $1}'); fi
[ "$psychotalk_actual" = "$psychotalk_expected" ] || { echo 'Checksum divergente. Instalação cancelada.' >&2; exit 1; }
tar -tzf "$psychotalk_package" > files.list
while IFS= read -r psychotalk_member; do
 psychotalk_member=${psychotalk_member%/}
 case "$psychotalk_member" in psychotalk|psychotalk/) ;; psychotalk/*) case "/$psychotalk_member/" in */../*|*/./*|*'//'*) echo 'Caminho inválido no pacote.' >&2; exit 1 ;; esac ;; *) echo 'Caminho inválido no pacote.' >&2; exit 1 ;; esac
done < files.list
tar -tvzf "$psychotalk_package" | awk 'substr($0,1,1)!="-" && substr($0,1,1)!="d" {exit 1}' || { echo 'Tipo de arquivo inválido no pacote.' >&2; exit 1; }
tar -xzf "$psychotalk_package"
[ -x psychotalk/psychotalk ] && [ -x psychotalk/libexec/tor ] && [ -f psychotalk/install.sh ] || { echo 'Pacote incompleto.' >&2; exit 1; }
if [ -L "$psychotalk_program" ] || { [ -e "$psychotalk_program" ] && { [ ! -f "$psychotalk_program/.owner" ] || [ "$(cat "$psychotalk_program/.owner")" != 'Psychotalk program v1' ]; }; }; then echo 'Diretório de programa não reconhecido; nenhum arquivo foi substituído.' >&2; exit 1; fi
mkdir -p "$psychotalk_program/versions" "$(dirname "$psychotalk_launcher")"
printf '%s\n' 'Psychotalk program v1' > "$psychotalk_program/.owner"
if [ -x "$psychotalk_program/current/psychotalk" ]; then "$psychotalk_program/current/psychotalk" stop; fi
if { [ -e "$psychotalk_launcher" ] && [ ! -L "$psychotalk_launcher" ]; } || { [ -L "$psychotalk_launcher" ] && [ "$(readlink "$psychotalk_launcher")" != "$psychotalk_program/current/psychotalk" ]; }; then echo 'Já existe um programa nesse caminho; nenhum arquivo foi substituído.' >&2; exit 1; fi
psychotalk_destination="$psychotalk_program/versions/$psychotalk_version-$psychotalk_system-$psychotalk_arch"
if [ -e "$psychotalk_destination" ]; then psychotalk_destination="$psychotalk_destination-$(date +%s)"; fi
mv psychotalk "$psychotalk_destination"
ln -s "$psychotalk_destination" "$psychotalk_program/current.next"
if [ "$psychotalk_system" = macos ]; then mv -fh "$psychotalk_program/current.next" "$psychotalk_program/current"; else mv -fT "$psychotalk_program/current.next" "$psychotalk_program/current"; fi
ln -s "$psychotalk_program/current/psychotalk" "$psychotalk_launcher.next"
if [ "$psychotalk_system" = macos ]; then mv -fh "$psychotalk_launcher.next" "$psychotalk_launcher"; else mv -fT "$psychotalk_launcher.next" "$psychotalk_launcher"; fi
"$psychotalk_launcher" --version
echo "Instalado. Para abrir: \"$psychotalk_launcher\""
echo 'Se ~/.local/bin já estiver no PATH, use: psychotalk'
echo 'Atualizar: psychotalk update | Remover programa mantendo cofre: psychotalk uninstall'
