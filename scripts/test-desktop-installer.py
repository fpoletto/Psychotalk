#!/usr/bin/env python3
"""Installer integration with the real native package; only the GitHub download is stubbed."""
from pathlib import Path
import hashlib,os,subprocess,sys,tempfile
package=Path(sys.argv[1]).resolve();script=Path(__file__).resolve().parent/'install.sh'
with tempfile.TemporaryDirectory(prefix='psychotalk-install-test-') as folder:
 root=Path(folder);mock=root/'mock';mock.mkdir();gh=mock/'gh'
 gh.write_text('''#!/usr/bin/env python3
import os,pathlib,shutil,sys
args=sys.argv[1:]
if args[:2]==['auth','status']:raise SystemExit(0)
if args[0]=='api':print('v0.4.2-experimental');raise SystemExit(0)
if args[:2]==['release','download']:
 target=pathlib.Path(args[args.index('--dir')+1]);target.mkdir(parents=True,exist_ok=True)
 package=pathlib.Path(os.environ['INSTALLER_FIXTURE_PACKAGE']);shutil.copyfile(package,target/package.name)
 (target/'SHA256SUMS').write_text(os.environ['INSTALLER_FIXTURE_HASH']+'  '+package.name+'\\n');raise SystemExit(0)
raise SystemExit('unexpected gh command')
''');gh.chmod(0o755)
 env=os.environ.copy();env.update(PATH=str(mock)+os.pathsep+env['PATH'],PSYCHOTALK_DATA_DIR=str(root/'data'),PSYCHOTALK_BIN_DIR=str(root/'bin'),INSTALLER_FIXTURE_PACKAGE=str(package),INSTALLER_FIXTURE_HASH=hashlib.sha256(package.read_bytes()).hexdigest())
 subprocess.run(['sh',str(script)],env=env,check=True)
 launcher=root/'bin/psychotalk';assert launcher.is_symlink();assert '0.4.2' in subprocess.check_output([str(launcher),'--version'],env=env,text=True)
 vault=root/'data/vault';vault.mkdir();(vault/'preserve.fixture').write_text('keep encrypted vault data')
 first=(root/'data/program/current').resolve()
 subprocess.run([str(launcher),'update'],env=env,check=True)
 assert (root/'data/program/current').resolve()!=first,'current symlink was not atomically replaced'
 assert (vault/'preserve.fixture').read_text()=='keep encrypted vault data'
 subprocess.run([str(launcher),'uninstall'],env=env,check=True)
 assert not launcher.exists();assert not (root/'data/program').exists();assert (vault/'preserve.fixture').exists()
 env['INSTALLER_FIXTURE_HASH']='0'*64
 rejected=subprocess.run(['sh',str(script)],env=env,capture_output=True,text=True)
 assert rejected.returncode!=0 and not launcher.exists(),'corrupt package was installed'
 print('PASS: real native package install/update/uninstall preserves vault; bad checksum rejected')
