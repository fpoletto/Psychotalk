#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mkdir -p .tools/downloads .tools/desktop-tor
python3 - <<'PY'
from pathlib import Path
import hashlib,urllib.request,tarfile
archive=Path('.tools/downloads/tor-desktop-0.4.9.14.tar.gz')
if not archive.exists():urllib.request.urlretrieve('https://dist.torproject.org/tor-0.4.9.14.tar.gz',archive)
if hashlib.sha256(archive.read_bytes()).hexdigest()!='182449ac1c8ff43278da27b69b02040e11d03d0327db6d9126e4190ec6235e4a':raise SystemExit('Tor checksum mismatch')
root=Path('.tools/desktop-tor')
if not (root/'tor-0.4.9.14').exists():
 with tarfile.open(archive) as tar:
  if hasattr(tarfile,'data_filter'):tar.extractall(root,filter='data')
  else:
   members=tar.getmembers()
   for member in members:
    path=Path(member.name)
    if path.is_absolute() or '..' in path.parts or not (member.isfile() or member.isdir()):raise SystemExit('Unsafe source archive')
   tar.extractall(root,members=members)
PY
cd .tools/desktop-tor/tor-0.4.9.14
if [ "$(uname -s)" = Darwin ]; then
  tor_openssl=$(brew --prefix openssl@3)
  tor_event=$(brew --prefix libevent)
  export CPPFLAGS="-I$tor_openssl/include -I$tor_event/include"
  export LDFLAGS="-L$tor_openssl/lib -L$tor_event/lib"
fi
if [ ! -f Makefile ]; then ./configure --disable-asciidoc --disable-unittests --disable-lzma --disable-zstd; fi
make -j4
./src/app/tor --version
