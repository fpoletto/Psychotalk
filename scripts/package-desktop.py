#!/usr/bin/env python3
"""Bundle the native executable, Tor, non-system libraries and third-party notices."""
import argparse,hashlib,json,os,pathlib,platform,re,shutil,subprocess,tarfile,tempfile
p=argparse.ArgumentParser();p.add_argument('--binary',default='target/release/psychotalk');p.add_argument('--tor',default='.tools/desktop-tor/tor-0.4.9.14/src/app/tor');p.add_argument('--output',default='dist');a=p.parse_args()
root=pathlib.Path(__file__).resolve().parent.parent;os.chdir(root)
version='0.4.2-experimental';system={'Darwin':'macos','Linux':'linux'}[platform.system()];arch={'arm64':'arm64','aarch64':'arm64','x86_64':'x86_64'}[platform.machine()]
assert (system,arch) in [('macos','arm64'),('macos','x86_64'),('linux','x86_64')]
output=pathlib.Path(a.output);output.mkdir(parents=True,exist_ok=True)
name=f'psychotalk-{version}-{system}-{arch}.tar.gz'
def run(*args):return subprocess.check_output(args,text=True)
def sha(file):return hashlib.sha256(pathlib.Path(file).read_bytes()).hexdigest()
with tempfile.TemporaryDirectory() as temp:
 package=pathlib.Path(temp)/'psychotalk';(package/'libexec').mkdir(parents=True);(package/'lib').mkdir();(package/'licenses').mkdir()
 binary=pathlib.Path(a.binary).resolve();tor=pathlib.Path(a.tor).resolve();shutil.copyfile(binary,package/'psychotalk');shutil.copyfile(tor,package/'libexec/tor')
 for f in [package/'psychotalk',package/'libexec/tor']:f.chmod(0o755)
 shutil.copyfile('scripts/install.sh',package/'install.sh');shutil.copyfile('docs/DESKTOP.md',package/'README.md');shutil.copyfile('docs/SECURITY.md',package/'SECURITY.md')
 queue=[(binary,package/'psychotalk'),(tor,package/'libexec/tor')];seen={};notices=[]
 while queue:
  source,dest=queue.pop(0)
  if system=='macos':deps=[line.strip().split(' (')[0] for line in run('otool','-L',str(source)).splitlines()[1:]]
  else:
   text=run('ldd',str(source));assert 'not found' not in text,text
   deps=re.findall(r'=>\s+(/\S+)',text)
  for dep in deps:
   if system=='macos':
    if dep.startswith(('/usr/lib/','/System/Library/')):continue
    assert dep.startswith('/'),f'Unresolved dependency {dep}'
   elif pathlib.Path(dep).name in ['libc.so.6','libm.so.6','libdl.so.2','libpthread.so.0','librt.so.1']:continue
   source_lib=pathlib.Path(dep).resolve();libname=pathlib.Path(dep).name;digest=sha(source_lib);libdest=package/'lib'/libname
   if libname not in seen:
    seen[libname]=digest;shutil.copyfile(source_lib,libdest);libdest.chmod(0o755);queue.append((source_lib,libdest))
    if system=='macos':
     parts=source_lib.parts
     if 'Cellar' in parts:
      n=parts.index('Cellar');prefix=pathlib.Path(*parts[:n+3])
      for pattern in ['LICENSE*','COPYING*','NOTICE*']:
       for license_file in prefix.rglob(pattern):
        if license_file.is_file():
         target=package/'licenses'/'native'/prefix.parent.name/license_file.relative_to(prefix);target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(license_file,target)
    else:
     result=subprocess.run(['dpkg-query','-S',str(source_lib)],capture_output=True,text=True)
     for line in result.stdout.splitlines():
      pkg=line.split(': ')[0].split(':')[0];license_file=pathlib.Path('/usr/share/doc')/pkg/'copyright'
      if license_file.is_file():shutil.copyfile(license_file,package/'licenses'/f'{pkg}-copyright')
   else:assert seen[libname]==digest,f'Library name collision: {libname}'
   if system=='macos':
    relative=os.path.relpath(libdest,dest.parent)
    subprocess.run(['install_name_tool','-change',dep,'@loader_path/'+relative,str(dest)],check=True,capture_output=True)
  if system=='linux':subprocess.run(['patchelf','--set-rpath','$ORIGIN' if dest.parent.name=='lib' else '$ORIGIN/'+os.path.relpath(package/'lib',dest.parent),str(dest)],check=True)
 if system=='macos':
  for file in [*(package/'lib').iterdir(),package/'libexec/tor',package/'psychotalk']:subprocess.run(['codesign','--force','--sign','-',str(file)],check=True,capture_output=True)
 metadata=json.loads(run('cargo','metadata','--locked','--format-version=1'))
 for dep in metadata['packages']:
  if not dep.get('source'):continue
  folder=pathlib.Path(dep['manifest_path']).parent
  for pattern in ['LICENSE*','COPYING*','NOTICE*']:
   for file in folder.rglob(pattern):
    if file.is_file():
     target=package/'licenses'/f"{dep['name']}-{dep['version']}"/file.relative_to(folder);target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(file,target)
 for file in tor.parents[2].glob('LICENSE*'):shutil.copyfile(file,package/'licenses'/('tor-'+file.name))
 # The Tor source directory is src/app/tor → src/app → src → project root.
 for file in tor.parents[2].glob('README*'):
  if file.is_file():shutil.copyfile(file,package/'licenses'/('tor-'+file.name))
 (package/'BUILD.json').write_text(json.dumps({'version':version,'system':system,'architecture':arch,'tor_version':'0.4.9.14','source_commit':run('git','rev-parse','HEAD').strip()},indent=2)+'\n')
 subprocess.run([str(package/'psychotalk'),'--version'],check=True);subprocess.run([str(package/'libexec/tor'),'--version'],check=True)
 with tarfile.open(output/name,'w:gz') as archive:archive.add(package,arcname='psychotalk')
 (output/f'SHA256SUMS-{system}-{arch}.txt').write_text(f'{sha(output/name)}  {name}\n')
 print(output/name)
